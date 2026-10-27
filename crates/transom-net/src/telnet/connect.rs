//! Telnet TCP 连接拨号、协商与控制线程组装。

use super::stream::{
    READ_CHUNK, READ_TIMEOUT, TelnetReader, TelnetSession, TelnetWire, TelnetWriter, encoding_for,
};
use crate::telnet_iac::{
    IAC, IacParser, NOP, OPT_NAWS, OPT_SGA, OPT_TTYPE, WILL, naws_subnegotiation,
};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use transom_core::{
    AppError, ErrorCode, TelnetConnection, TerminalControl, TerminalControlHandle, TerminalIo,
    TerminalSize,
};

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
                    last_keepalive = std::time::Instant::now();
                    if control_wire.send_raw(&[IAC, NOP]).is_err() {
                        break;
                    }
                }
                std::thread::sleep(READ_TIMEOUT);
            }
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

    log::info!(
        "Telnet 会话已建立，主机: {host}，端口: {}，编码: {:?}",
        connection.port,
        connection.encoding
    );
    Ok(TelnetSession { io, exit: exit_rx })
}

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
