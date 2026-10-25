//! Telnet 终端传输实现（RFC 854/855 最小客户端）。
//!
//! 与串口传输同口径：阻塞读写、100ms 超时轮询保证关闭及时生效。协议命令
//! 的剥离与转义在 `telnet_iac` 状态机内完成，本模块只负责三件事：拨号与
//! 会话装配、控制消息（窗口尺寸/保活/关闭）到协议报文的转换、按会话编码
//! 在字节流边界转码。终端仿真层始终只面对 UTF-8 数据字节流。

use crate::telnet_iac::{
    IAC, IacParser, NOP, OPT_NAWS, OPT_SGA, OPT_TTYPE, WILL, escape_iac, naws_subnegotiation,
};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use transom_core::{
    AppError, ErrorCode, TelnetConnection, TerminalControl, TerminalControlHandle, TerminalIo,
    TerminalSize, TextEncoding,
};

/// 读取超时；同时决定关闭请求的最长响应延迟（与串口同口径）。
const READ_TIMEOUT: Duration = Duration::from_millis(100);
/// 读块大小；编码解码后最多膨胀 3 倍（GBK 双字节到 UTF-8 三字节）。
const READ_CHUNK: usize = 4096;

/// 已建立的 Telnet 终端通道。
pub struct TelnetSession {
    pub io: TerminalIo,
    /// 读取端退出（对端断开、关闭或读错误）时触发。
    pub exit: tokio::sync::oneshot::Receiver<()>,
}

/// 线路写出端：用户数据、协商应答、NAWS 与保活 NOP 的唯一出口。
///
/// Telnet 是字符流协议，命令允许插在数据字符之间，唯一约束是单条报文
/// 不被中途交错；互斥锁只保护单次 `write_all`，临界区内没有嵌套锁与
/// 长时间计算。锁中毒按可恢复处理：数据本体是 TCP 流，panic 不会破坏
/// 其内部一致性，直接取回内部值继续使用。
struct TelnetWire {
    inner: Mutex<Box<dyn Write + Send>>,
}

impl TelnetWire {
    fn send_raw(&self, bytes: &[u8]) -> io::Result<()> {
        self.lock().write_all(bytes)
    }

    /// 统一处理锁中毒：数据本体是 TCP 流，panic 不会破坏其内部一致性，
    /// 直接取回内部值继续使用，不向调用方扩散毒化错误。
    fn lock(&self) -> MutexGuard<'_, Box<dyn Write + Send>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Telnet 读取端：剥离协议命令、按会话编码转码为 UTF-8。
struct TelnetReader {
    stream: Box<dyn Read + Send>,
    wire: Arc<TelnetWire>,
    parser: IacParser,
    /// `None` 表示 UTF-8 直通；其余编码持有跨块保留部分序列状态的解码器。
    decoder: Option<encoding_rs::Decoder>,
    /// 协议剥离后的数据字节（远端编码），跨轮次复用避免高频重分配。
    clean: Vec<u8>,
    /// 转码后的 UTF-8 字节及其读取偏移；单次 read 取不完时留下次续取。
    pending: Vec<u8>,
    offset: usize,
    /// 解码用的字符串缓冲，与 `pending` 同生命周期复用。
    decoded: String,
    closed: Arc<AtomicBool>,
    exit: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Read for TelnetReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            if self.offset < self.pending.len() {
                let count = buffer.len().min(self.pending.len() - self.offset);
                buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
                self.offset += count;
                if self.offset == self.pending.len() {
                    self.pending.clear();
                    self.offset = 0;
                }
                return Ok(count);
            }

            let mut chunk = [0u8; READ_CHUNK];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(0),
                Ok(count) => {
                    let wire = self.wire.clone();
                    self.parser
                        .feed(&chunk[..count], &mut self.clean, &mut |bytes| {
                            // 协议应答写失败意味着线路已断，随后读取也会出错，
                            // 这里仅记录告警，不中断当前批次的数据剥离。
                            if let Err(error) = wire.send_raw(bytes) {
                                tracing::warn!("写入 Telnet 协议应答失败: {error}");
                            }
                        });
                    if self.clean.is_empty() {
                        continue;
                    }
                    self.decode_clean();
                }
                // 超时是被预期的空转：回到循环头重新检查关闭标志。
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl TelnetReader {
    /// 把剥离后的数据字节按会话编码转成 UTF-8 并排入待取缓冲。
    ///
    /// 非法字节由 encoding_rs 替换为 U+FFFD（终端领域的标准容错），
    /// 不中断会话；逐块解码不产生日志，避免高频热点打日志。
    fn decode_clean(&mut self) {
        match self.decoder.as_mut() {
            Some(decoder) => {
                self.decoded.clear();
                // decode_to_string 以 String 现有容量为输出上限且不自行扩容，
                // 必须循环消费：OutputFull 时扩容重试剩余输入，直至读完。
                let mut consumed = 0;
                while consumed < self.clean.len() {
                    let (result, read, _) =
                        decoder.decode_to_string(&self.clean[consumed..], &mut self.decoded, false);
                    consumed += read;
                    if matches!(result, encoding_rs::CoderResult::OutputFull) {
                        self.decoded.reserve(self.clean.len() * 3 + 8);
                    } else {
                        break;
                    }
                }
                self.pending.extend_from_slice(self.decoded.as_bytes());
            }
            None => self.pending.extend_from_slice(&self.clean),
        }
        self.clean.clear();
    }
}

impl Drop for TelnetReader {
    fn drop(&mut self) {
        if let Some(exit) = self.exit.take() {
            let _ = exit.send(());
        }
    }
}

/// Telnet 写入端：按会话编码转码并转义 0xFF 后写线路。
struct TelnetWriter {
    wire: Arc<TelnetWire>,
    /// `None` 表示 UTF-8 直通；其余编码持有跨写入复用的编码器。
    encoder: Option<encoding_rs::Encoder>,
    /// 编码产物与转义产物，跨轮次复用。
    encoded: Vec<u8>,
    escaped: Vec<u8>,
}

impl Write for TelnetWriter {
    /// 全量消费语义：要么整批转换并写线路成功，要么整批报错。
    /// 调用方（终端写线程）统一使用 `write_all`，单次全量返回即可。
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        match self.encoder.as_mut() {
            Some(encoder) => {
                let text = std::str::from_utf8(buffer).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "终端输入不是合法 UTF-8，无法按会话编码转码",
                    )
                })?;
                self.encoded.clear();
                self.encoded.reserve(text.len() * 3 + 8);
                encode_input(encoder, text, &mut self.encoded);
                self.escaped.clear();
                escape_iac(&self.encoded, &mut self.escaped);
            }
            None => {
                self.escaped.clear();
                escape_iac(buffer, &mut self.escaped);
            }
        }
        self.wire.send_raw(&self.escaped)?;
        Ok(buffer.len())
    }

    /// TCP 写入直接进入内核缓冲，无用户态缓冲需要冲刷。
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 把 UTF-8 文本编码为会话编码字节。
///
/// 逐段循环直到源文本全部消费：输出缓冲不足时扩容重试，不可映射字符
/// 由 encoding_rs 落成数字字符引用（已消费，继续后续文本）。
fn encode_input(encoder: &mut encoding_rs::Encoder, text: &str, out: &mut Vec<u8>) {
    let mut consumed = 0;
    while consumed < text.len() {
        let (result, read, _had_unmappable) =
            encoder.encode_from_utf8_to_vec(&text[consumed..], out, true);
        consumed += read;
        match result {
            encoding_rs::CoderResult::InputEmpty => break,
            encoding_rs::CoderResult::OutputFull => out.reserve(text.len() * 3 + 8),
        }
    }
}

/// 打开 Telnet 连接并构造与终端控件对接的字节流。
///
/// # Errors
/// 主机地址为空、端口为 0、域名解析无结果、连接超时或被拒绝、建立初始
/// 协商失败时返回 `AppError`，错误文本携带主机与端口现场。
pub fn connect_telnet(
    connection: &TelnetConnection,
    size: TerminalSize,
) -> Result<TelnetSession, AppError> {
    let host = connection.host.trim();
    if host.is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "Telnet 主机地址不能为空",
        ));
    }
    if connection.port == 0 {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "Telnet 端口必须大于 0",
        ));
    }

    let timeout = Duration::from_secs(connection.options.connect_timeout_secs.max(1));
    let stream = dial(host, connection.port, timeout)?;
    stream.set_nodelay(true).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!("配置 Telnet 连接失败，主机: {host}，原因: {error}"),
        )
    })?;
    // 读超时只影响读端语义；写端随后被 move 走，不再依赖超时。
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!("配置 Telnet 连接失败，主机: {host}，原因: {error}"),
            )
        })?;
    let write_stream = stream.try_clone().map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!("复制 Telnet 写句柄失败，主机: {host}，原因: {error}"),
        )
    })?;

    let wire = Arc::new(TelnetWire {
        inner: Mutex::new(Box::new(write_stream)),
    });
    let closed = Arc::new(AtomicBool::new(false));
    let (control, mut control_rx) = TerminalControlHandle::channel(16);

    // 控制线程：窗口尺寸转 NAWS 子协商、按需周期保活、关闭时唤醒阻塞读。
    let control_wire = wire.clone();
    let control_closed = closed.clone();
    let keepalive = if connection.options.keepalive_secs > 0 {
        Some(Duration::from_secs(connection.options.keepalive_secs))
    } else {
        None
    };
    std::thread::Builder::new()
        .name("transom-telnet-control".into())
        .spawn(move || {
            // tokio mpsc 没有阻塞线程可用的超时接收，这里以 100ms 轮询
            // 消化控制消息并按墙钟计时保活；轮询间隔与串口读取空转同
            // 口径，保活周期通常为秒级，轮询开销可忽略。
            let mut last_keepalive = std::time::Instant::now();
            loop {
                match control_rx.try_recv() {
                    Ok(TerminalControl::Resize(size)) => {
                        let naws = naws_subnegotiation(size.cols, size.rows);
                        if control_wire.send_raw(&naws).is_err() {
                            break;
                        }
                    }
                    Ok(TerminalControl::Close) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                }
                if let Some(interval) = keepalive
                    && last_keepalive.elapsed() >= interval
                {
                    // 保活周期到点：发送 NOP 维持 NAT 映射与连接探活。
                    last_keepalive = std::time::Instant::now();
                    if control_wire.send_raw(&[IAC, NOP]).is_err() {
                        break;
                    }
                }
                std::thread::sleep(READ_TIMEOUT);
            }
            // Close 或控制通道断开都意味着终端视图已退出，唤醒阻塞读。
            control_closed.store(true, Ordering::Release);
        })
        .map_err(|error| AppError::with_detail(ErrorCode::Io, error.to_string()))?;

    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    let encoding = encoding_for(connection.encoding);
    let reader = TelnetReader {
        stream: Box::new(stream),
        wire: wire.clone(),
        parser: IacParser::new(),
        decoder: encoding.map(encoding_rs::Encoding::new_decoder),
        clean: Vec::with_capacity(READ_CHUNK * 3),
        pending: Vec::with_capacity(READ_CHUNK * 3),
        offset: 0,
        decoded: String::with_capacity(READ_CHUNK * 3),
        closed: closed.clone(),
        exit: Some(exit_tx),
    };
    let writer = TelnetWriter {
        wire: wire.clone(),
        encoder: encoding.map(encoding_rs::Encoding::new_encoder),
        encoded: Vec::with_capacity(READ_CHUNK),
        escaped: Vec::with_capacity(READ_CHUNK),
    };
    let io = TerminalIo::new(Box::new(reader), Box::new(writer), control);

    // 连接建立即申报客户端能力并上报初始窗口尺寸；未获远端 DO 许可前
    // 发送 NAWS 属于宽容做法，主流实现（telnetd/lrzsz/BBS）均按忽略处理。
    let mut offer = Vec::with_capacity(12);
    offer.extend_from_slice(&[IAC, WILL, OPT_SGA]);
    offer.extend_from_slice(&[IAC, WILL, OPT_TTYPE]);
    offer.extend_from_slice(&[IAC, WILL, OPT_NAWS]);
    wire.send_raw(&offer).map_err(|error| {
        AppError::with_detail(
            ErrorCode::ConnectionFailed,
            format!("发送 Telnet 初始协商失败，主机: {host}，原因: {error}"),
        )
    })?;
    let naws = naws_subnegotiation(size.cols, size.rows);
    wire.send_raw(&naws).map_err(|error| {
        AppError::with_detail(
            ErrorCode::ConnectionFailed,
            format!("发送 Telnet 窗口尺寸失败，主机: {host}，原因: {error}"),
        )
    })?;

    tracing::info!(
        host = host,
        port = connection.port,
        encoding = ?connection.encoding,
        "Telnet 会话已建立"
    );
    Ok(TelnetSession { io, exit: exit_rx })
}

/// 依次尝试解析出的全部地址拨号（域名可能对应多个 A 记录）。
///
/// # Errors
/// 域名解析失败、无可用地址或全部地址连接失败（含超时）时返回带现场
/// 信息的 `AppError`。
fn dial(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, AppError> {
    let resolved: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::ConnectionFailed,
                format!("解析 Telnet 主机失败，主机: {host}，端口: {port}，原因: {error}"),
            )
        })?
        .collect();
    if resolved.is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::ConnectionFailed,
            format!("Telnet 主机没有可用地址，主机: {host}，端口: {port}"),
        ));
    }

    let mut last_error = None;
    for address in resolved {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    let reason = last_error.map_or_else(|| "未知原因".to_string(), |error| error.to_string());
    Err(AppError::with_detail(
        ErrorCode::ConnectionFailed,
        format!("连接 Telnet 服务失败，主机: {host}，端口: {port}，原因: {reason}"),
    ))
}

fn encoding_for(encoding: TextEncoding) -> Option<&'static encoding_rs::Encoding> {
    match encoding {
        TextEncoding::Utf8 => None,
        TextEncoding::Gbk => Some(encoding_rs::GBK),
        TextEncoding::Big5 => Some(encoding_rs::BIG5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex;

    /// 测试用线路汇聚：把写出的字节收集到共享缓冲供断言。
    #[derive(Clone)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl SharedSink {
        fn new() -> (Self, Arc<Mutex<Vec<u8>>>) {
            let shared = Arc::new(Mutex::new(Vec::new()));
            (Self(shared.clone()), shared)
        }
    }

    impl Write for SharedSink {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn wire_with_sink(sink: SharedSink) -> Arc<TelnetWire> {
        Arc::new(TelnetWire {
            inner: Mutex::new(Box::new(sink)),
        })
    }

    /// 反复读取直到流耗尽，汇总全部输出（解析器可能分多次返回）。
    fn drain_reader(reader: &mut TelnetReader) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        let mut buffer = [0u8; READ_CHUNK];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) => return Err(error),
            }
        }
        Ok(output)
    }

    #[test]
    fn reader_strips_negotiation_and_decodes_gbk() {
        // "登录" 的 GBK 字节。
        let source = [&[IAC, 253u8, OPT_TTYPE][..], &[0xb5, 0xc7, 0xc2, 0xbc]].concat();
        let (sink, shared) = SharedSink::new();
        let closed = Arc::new(AtomicBool::new(false));
        let (exit_tx, _exit_rx) = tokio::sync::oneshot::channel::<()>();
        let mut reader = TelnetReader {
            stream: Box::new(Cursor::new(source)),
            wire: wire_with_sink(sink),
            parser: IacParser::new(),
            decoder: Some(encoding_rs::GBK.new_decoder()),
            clean: Vec::new(),
            pending: Vec::new(),
            offset: 0,
            decoded: String::new(),
            closed,
            exit: Some(exit_tx),
        };
        let output = drain_reader(&mut reader).expect("读取不应失败");
        assert_eq!(String::from_utf8_lossy(&output), "登录");
        // DO TTYPE 得到 WILL 应答。
        let written = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(written, vec![IAC, WILL, OPT_TTYPE]);
    }

    #[test]
    fn writer_encodes_gbk() {
        let (sink, shared) = SharedSink::new();
        let mut writer = TelnetWriter {
            wire: wire_with_sink(sink),
            encoder: Some(encoding_rs::GBK.new_encoder()),
            encoded: Vec::new(),
            escaped: Vec::new(),
        };
        writer.write_all("密码".as_bytes()).expect("写入不应失败");
        let written = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        // "密码" 的 GBK 字节；GBK 输出不含 0xFF，转义为恒等。
        assert_eq!(written, vec![0xc3, 0xdc, 0xc2, 0xeb]);
    }

    #[test]
    fn writer_rejects_non_utf8_input() {
        let (sink, _shared) = SharedSink::new();
        let mut writer = TelnetWriter {
            wire: wire_with_sink(sink),
            encoder: Some(encoding_rs::GBK.new_encoder()),
            encoded: Vec::new(),
            escaped: Vec::new(),
        };
        let error = writer.write(&[0xff, 0xfe]).expect_err("非法 UTF-8 应报错");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn utf8_passthrough_only_escapes() {
        let (sink, shared) = SharedSink::new();
        let mut writer = TelnetWriter {
            wire: wire_with_sink(sink),
            encoder: None,
            encoded: Vec::new(),
            escaped: Vec::new(),
        };
        writer.write_all(b"ok\xff").expect("写入不应失败");
        let written = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(written, vec![b'o', b'k', IAC, IAC]);
    }

    #[test]
    fn encoding_for_maps_all_variants() {
        assert!(encoding_for(TextEncoding::Utf8).is_none());
        assert_eq!(encoding_for(TextEncoding::Gbk), Some(encoding_rs::GBK));
        assert_eq!(encoding_for(TextEncoding::Big5), Some(encoding_rs::BIG5));
    }
}
