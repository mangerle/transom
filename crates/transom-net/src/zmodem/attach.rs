//! ZMODEM 监督层：把协议状态机装配到 `TerminalIo` 上的读/写过滤与驱动线程。
//!
//! 设计原理（Why）：
//! - 读方向空闲时透传终端渲染，同时扫描 ZMODEM 发起标记（`**` + ZDLE +
//!   格式字节）；命中后原始字节转入专用通道交状态机消费，终端不再看到
//!   协议噪声。会话结束把通道残余经回注通道还给终端（如 shell 提示符），
//!   过滤层随即回到空闲扫描——原子标志 Release/Acquire 保序。
//! - 写方向在会话激活期丢弃用户键入（此时对端只认协议字节），空闲期透传；
//!   真实 writer 以 `Arc<Mutex<..>>` 与驱动线程共享，临界区仅一次 write_all。
//! - 状态机驱动在专属阻塞线程（"transom-zmodem-fsm"），文件 I/O 与协议
//!   解码都不占用异步运行时；线程随过滤层析构（通道断开）自然退出。
//! - 命令通道有界 4 且 `try_send` 快速失败：同标签连点两次上传只保留
//!   第一个决策，绝不积压。

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use transom_core::{TerminalIo, ZmodemEvent, ZmodemFinished};

use super::frames::{ZBIN32, ZDLE, ZHEX};
use super::fsm::{Fsm, FsmChannels, ZmodemCommand, fsm_loop};

/// 读过滤层单次自底层读取的块大小。
const READ_CHUNK: usize = 8192;
/// 发起标记长度：`*` `*` ZDLE 格式字节。
const MARK_LEN: usize = 4;
/// 转发块的等待窗口：驱动线程短暂无响应时重查会话状态，避免读线程
/// 因无人消费而永久卡死。
const FORWARD_WAIT: Duration = Duration::from_secs(1);
/// 原始字节通道容量（块粒度）：约 2 MiB 缓冲，打满时读线程阻塞形成背压。
const RAW_CAPACITY: usize = 256;
/// 回注通道容量：会话结束后的终端残余字节（通常一两块）。
const REPLAY_CAPACITY: usize = 64;
/// 命令通道容量：上传决策与取消，低频命令。
const COMMAND_CAPACITY: usize = 4;

/// ZMODEM 监督层配置。
#[derive(Debug, Clone)]
pub struct ZmodemConfig {
    /// 下载落盘目录（调用方已解析系统目录兜底）。
    pub download_dir: PathBuf,
    /// 对端 sz 发起下载时是否自动接收。
    pub auto_receive: bool,
}

/// 界面侧的 ZMODEM 控制句柄；通道满时命令被丢弃并返回假（快速失败）。
#[derive(Clone)]
pub struct ZmodemControl {
    tx: flume::Sender<ZmodemCommand>,
}

impl ZmodemControl {
    /// 提交上传决策；会话不处于等待状态或通道满时返回假。
    pub fn upload(&self, files: Vec<PathBuf>) -> bool {
        if files.is_empty() {
            return false;
        }
        self.tx.try_send(ZmodemCommand::Upload(files)).is_ok()
    }

    /// 请求取消当前传输；无会话时返回假（静默忽略即可）。
    pub fn cancel(&self) -> bool {
        self.tx.try_send(ZmodemCommand::Cancel).is_ok()
    }
}

/// 过滤层与驱动线程共享的会话状态。
pub(crate) struct Shared {
    /// 协议会话是否激活：激活期读侧吞掉协议字节、写侧丢弃用户键入。
    pub(crate) active: AtomicBool,
    /// 驱动线程是否已退出（正常收尾或 panic 展开）。终止后过滤层不再接管
    /// 字节流，否则协议通道无人消费，终端表现为输出冻结、键入全部丢失。
    pub(crate) terminated: AtomicBool,
}

/// 驱动线程的存活凭证：线程函数退出时（含 panic 展开）自动复位共享状态。
///
/// 设计原理：用 RAII 而非 `JoinHandle` 轮询，是因为读线程与驱动线程分属两侧，
/// 任何退出路径都必须复位标志——漏掉一条就是整个终端不可用。panic 时额外上报
/// 失败结局，避免界面徽章停在"传输中"无法清除。
struct FsmLifeline {
    shared: Arc<Shared>,
    events: flume::Sender<ZmodemEvent>,
}

impl Drop for FsmLifeline {
    fn drop(&mut self) {
        let panicked = std::thread::panicking();
        self.shared.active.store(false, Ordering::Release);
        self.shared.terminated.store(true, Ordering::Release);
        if panicked {
            log::error!("ZMODEM 驱动线程异常退出，本连接文件传输已停用");
            let _ = self
                .events
                .send(ZmodemEvent::Finished(ZmodemFinished::Failed(
                    "ZMODEM 传输线程异常退出，文件传输已停用".to_string(),
                )));
        }
    }
}

/// 读过滤层：空闲期扫描发起标记并透传；激活期把原始字节转入协议通道。
struct ZmodemReader {
    inner: Box<dyn Read + Send>,
    shared: Arc<Shared>,
    raw_tx: flume::Sender<Vec<u8>>,
    replay_rx: flume::Receiver<Vec<u8>>,
    /// 跨块保留的尾部字节（标记可能被数据块边界切开）。
    tail: Vec<u8>,
    /// 待返回给终端的字节（回注数据或标记前缀）。
    pending: Vec<u8>,
    /// 底层读取复用缓冲，避免每块堆分配。
    chunk: Vec<u8>,
    /// 底层已到 EOF（连接关闭）。
    eof: bool,
}

impl ZmodemReader {
    fn new(
        inner: Box<dyn Read + Send>,
        shared: Arc<Shared>,
        raw_tx: flume::Sender<Vec<u8>>,
        replay_rx: flume::Receiver<Vec<u8>>,
    ) -> Self {
        Self {
            inner,
            shared,
            raw_tx,
            replay_rx,
            tail: Vec::with_capacity(MARK_LEN - 1),
            pending: Vec::with_capacity(READ_CHUNK),
            chunk: Vec::with_capacity(READ_CHUNK),
            eof: false,
        }
    }

    /// 从底层读一块并推进扫描/转发，把终端可见部分写入 pending。
    fn pump(&mut self) -> io::Result<()> {
        self.chunk.resize(READ_CHUNK, 0);
        let count = self.inner.read(&mut self.chunk)?;
        if count == 0 {
            self.eof = true;
            return Ok(());
        }
        if self.shared.active.load(Ordering::Acquire) {
            self.chunk.truncate(count);
            let block = std::mem::take(&mut self.chunk);
            self.forward(block);
            return Ok(());
        }
        // 空闲期：拼接跨块尾部后扫描发起标记。
        let mut joined = std::mem::take(&mut self.tail);
        joined.extend_from_slice(&self.chunk[..count]);
        self.chunk.clear();
        match find_mark(&joined) {
            Some(_) if self.shared.terminated.load(Ordering::Acquire) => {
                // 驱动线程已退出：标记不再代表协议发起，整段按普通终端输出透传。
                self.pending = joined;
            }
            Some(position) => {
                self.pending = joined[..position].to_vec();
                let remainder = joined[position..].to_vec();
                // 先置激活再转发：驱动线程由此开始接管全部后续字节。
                self.shared.active.store(true, Ordering::Release);
                self.forward(remainder);
            }
            None => {
                let keep_from = joined.len() - pending_prefix_len(&joined);
                self.tail = joined[keep_from..].to_vec();
                self.pending = joined[..keep_from].to_vec();
            }
        }
        Ok(())
    }

    /// 激活期把原始块交给驱动线程；会话结束（标志翻假）后剩余数据回到
    /// 终端通路，读线程绝不因无人消费而永久卡死。
    fn forward(&mut self, mut block: Vec<u8>) {
        while self.shared.active.load(Ordering::Acquire)
            && !self.shared.terminated.load(Ordering::Acquire)
        {
            match self.raw_tx.send_timeout(block, FORWARD_WAIT) {
                Ok(()) => return,
                Err(flume::SendTimeoutError::Timeout(again)) => block = again,
                Err(flume::SendTimeoutError::Disconnected(again)) => {
                    block = again;
                    break;
                }
            }
        }
        self.pending.extend_from_slice(&block);
    }
}

impl Read for ZmodemReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pending.is_empty() {
            // 会话结束后的终端残余字节优先回注。
            match self.replay_rx.try_recv() {
                Ok(bytes) => self.pending = bytes,
                Err(_) if self.eof => return Ok(0),
                Err(_) => self.pump()?,
            }
        }
        let take = self.pending.len().min(buf.len());
        buf[..take].copy_from_slice(&self.pending[..take]);
        self.pending.drain(..take);
        Ok(take)
    }
}

/// 写过滤层：会话激活期丢弃用户键入（对端只认协议字节），空闲期透传。
struct ZmodemWriter {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
    shared: Arc<Shared>,
}

impl Write for ZmodemWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.shared.active.load(Ordering::Acquire) {
            return Ok(buf.len());
        }
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.flush()
    }
}

/// 发起标记扫描：`**` + ZDLE + hex（'B'）或二进制 32（'C'）格式字节。
fn find_mark(data: &[u8]) -> Option<usize> {
    if data.len() < MARK_LEN {
        return None;
    }
    (0..=data.len() - MARK_LEN).find(|&start| {
        data[start] == b'*'
            && data[start + 1] == b'*'
            && data[start + 2] == ZDLE
            && (data[start + 3] == ZHEX || data[start + 3] == ZBIN32)
    })
}

/// 数据尾部与标记前缀的最长匹配长度：仅这部分字节需要跨块保留，
/// 普通输出零延迟透传。
fn pending_prefix_len(data: &[u8]) -> usize {
    const MARK: [u8; MARK_LEN] = [b'*', b'*', ZDLE, ZHEX];
    (1..=data.len().min(MARK_LEN))
        .rev()
        .find(|&candidate| data[data.len() - candidate..] == MARK[..candidate])
        .unwrap_or(0)
}

/// 把 ZMODEM 监督层挂接到连接 IO 上，返回包装后的 IO 与控制句柄。
///
/// 设计权衡：读过滤层在空闲期对每个字节多做一次标记比对（纳秒级），换取
/// 全传输（SSH/本地/串口/Telnet）统一获得 ZMODEM 能力；写过滤层仅在会话
/// 激活期拦截键入，日常零开销。驱动线程创建失败属系统级异常：就地降级为
/// 禁用文件传输，终端功能不受影响（非致命错误不向连接流程扩散）。
pub fn attach_zmodem(
    io: TerminalIo,
    config: ZmodemConfig,
    events: flume::Sender<ZmodemEvent>,
) -> (TerminalIo, ZmodemControl) {
    let TerminalIo {
        reader,
        writer,
        control,
    } = io;
    let shared = Arc::new(Shared {
        active: AtomicBool::new(false),
        terminated: AtomicBool::new(false),
    });
    let writer = Arc::new(Mutex::new(writer));
    let (raw_tx, raw_rx) = flume::bounded::<Vec<u8>>(RAW_CAPACITY);
    let (replay_tx, replay_rx) = flume::bounded::<Vec<u8>>(REPLAY_CAPACITY);
    let (ctrl_tx, ctrl_rx) = flume::bounded::<ZmodemCommand>(COMMAND_CAPACITY);

    let reader = ZmodemReader::new(reader, Arc::clone(&shared), raw_tx, replay_rx);
    let writer_filter = ZmodemWriter {
        inner: Arc::clone(&writer),
        shared: Arc::clone(&shared),
    };
    let fsm = Fsm::new(
        FsmChannels {
            writer: Arc::clone(&writer),
            events: events.clone(),
            replay_tx,
            raw_rx,
            ctrl_rx,
            shared: Arc::clone(&shared),
        },
        config,
    );
    if let Err(error) = std::thread::Builder::new()
        .name("transom-zmodem-fsm".to_string())
        .spawn(move || {
            // 存活凭证先建立：无论 fsm_loop 正常返回还是 panic 展开，Drop 都会
            // 复位激活标志，杜绝"线程已死、过滤层继续吞字节"的终端冻结。
            let _lifeline = FsmLifeline {
                shared: Arc::clone(&shared),
                events,
            };
            fsm_loop(fsm);
        })
    {
        log::error!("ZMODEM 驱动线程创建失败，本连接禁用文件传输: {error}");
    }

    (
        TerminalIo {
            reader: Box::new(reader),
            writer: Box::new(writer_filter),
            control,
        },
        ZmodemControl { tx: ctrl_tx },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::frames::{
        ZCRCE, ZCRCG, ZCRCW, ZDLE, ZEOF, ZFILE, ZFIN, ZRINIT, ZRPOS, ZSINIT, build_bin16_header,
        build_bin32_header, build_hex_header, build_subpacket16, build_subpacket32, receiver_flags,
    };
    use crate::zmodem::fsm::Mode;
    use std::io::Cursor;
    use transom_core::ZmodemDirection;

    /// 测试事件通道容量：进度按子包粒度产出，64 足以平滑突发。
    const EVENT_CAPACITY: usize = 64;

    /// 写线路测试桩：收集全部字节。
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("测试桩不中毒").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// 按预设分块返回数据的底层读取桩（跨块标记测试用）。
    struct StepReader {
        chunks: Vec<Vec<u8>>,
        index: usize,
    }

    impl Read for StepReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.get(self.index) else {
                return Ok(0);
            };
            self.index += 1;
            let take = chunk.len().min(buf.len());
            buf[..take].copy_from_slice(&chunk[..take]);
            Ok(take)
        }
    }

    struct Harness {
        reader: ZmodemReader,
        fsm: Fsm,
        written: Arc<Mutex<Vec<u8>>>,
        events: flume::Receiver<ZmodemEvent>,
        ctrl_tx: flume::Sender<ZmodemCommand>,
        replay_tx: flume::Sender<Vec<u8>>,
    }

    fn harness(download_dir: PathBuf, auto_receive: bool) -> Harness {
        let shared = Arc::new(Shared {
            active: AtomicBool::new(false),
            terminated: AtomicBool::new(false),
        });
        let written = Arc::new(Mutex::new(Vec::new()));
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(SharedSink(Arc::clone(&written)))));
        let (raw_tx, raw_rx) = flume::bounded::<Vec<u8>>(RAW_CAPACITY);
        let (replay_tx, replay_rx) = flume::bounded::<Vec<u8>>(REPLAY_CAPACITY);
        let (ctrl_tx, ctrl_rx) = flume::bounded::<ZmodemCommand>(COMMAND_CAPACITY);
        let (event_tx, event_rx) = flume::bounded::<ZmodemEvent>(EVENT_CAPACITY);
        let reader = ZmodemReader::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Arc::clone(&shared),
            raw_tx,
            replay_rx,
        );
        let fsm = Fsm::new(
            FsmChannels {
                writer: Arc::clone(&writer),
                events: event_tx,
                replay_tx: replay_tx.clone(),
                raw_rx,
                ctrl_rx,
                shared: Arc::clone(&shared),
            },
            ZmodemConfig {
                download_dir,
                auto_receive,
            },
        );
        Harness {
            reader,
            fsm,
            written,
            events: event_rx,
            ctrl_tx,
            replay_tx,
        }
    }

    impl Harness {
        fn ctrl(&mut self, command: ZmodemCommand) -> bool {
            if self.ctrl_tx.try_send(command).is_ok() {
                if let Ok(command) = self.fsm.ctrl_rx.try_recv() {
                    self.fsm.handle_command(command);
                }
                return true;
            }
            false
        }

        fn wire(&mut self) -> Vec<u8> {
            std::mem::take(&mut *self.written.lock().expect("测试桩不中毒"))
        }

        fn collect_events(&mut self) -> Vec<ZmodemEvent> {
            std::iter::from_fn(|| self.events.try_recv().ok()).collect()
        }
    }

    #[test]
    fn reader_forwards_plain_data_until_mark() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.reader.inner = Box::new(Cursor::new(b"hello world".to_vec()));
        let mut out = [0u8; 64];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"hello world");
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
    }

    /// 会话结束后驱动线程回注的残留字节必须先于底层数据交付终端，
    /// 否则被协议层吞掉的最后一段输出会永久丢失（终端显示缺字）。
    #[test]
    fn replayed_tail_precedes_inner_reader() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .replay_tx
            .try_send(b"shell$ ".to_vec())
            .expect("回注通道有界可用");
        harness.reader.inner = Box::new(Cursor::new(b"prompt".to_vec()));
        let mut out = [0u8; 32];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"shell$ ", "回注字节优先输出");
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"prompt", "回注排空后底层数据照常透传");
    }

    /// 对端把 ZFIN/"OO" 与后续 shell 提示符挤在同一次 burst 送达时，
    /// 收尾之后的字节必须回到终端通路，不能被协议层当作残余吞掉。
    #[test]
    fn tail_of_finishing_block_returns_to_terminal() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness.fsm.mode = Mode::Download;
        let block = [build_bin16_header(ZFIN, [0, 0, 0, 0]), b"OO$ ".to_vec()].concat();
        harness.fsm.consume_block(block);
        assert!(matches!(harness.fsm.mode, Mode::Idle));
        let mut out = [0u8; 32];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"$ ", "会话结束后的同块残余字节回注终端");
    }

    #[test]
    fn reader_activates_on_sz_mark_and_hides_protocol_bytes() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        let payload = b"rz\r\n**\x18B00000000000000\r".to_vec();
        harness.reader.inner = Box::new(Cursor::new(payload));
        let mut out = [0u8; 64];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"rz\r\n", "标记前的终端回显照常透传");
        assert!(harness.reader.shared.active.load(Ordering::Acquire));
        // 协议字节已全部进通道，底层 EOF。
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(count, 0);
        let block = harness.fsm.raw_rx.recv().expect("标记块已入通道");
        assert!(block.starts_with(b"**\x18B"));
    }

    #[test]
    fn reader_mark_split_across_chunks_is_detected() {
        let full = b"**\x18B0100000023".to_vec();
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.reader.inner = Box::new(StepReader {
            chunks: vec![full[..3].to_vec(), full[3..].to_vec()],
            index: 0,
        });
        // 第一块只有标记前半：整体进入尾部缓存，不激活。
        harness.reader.pump().expect("读取成功");
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert_eq!(harness.reader.tail, b"**\x18");
        // 第二块补齐标记：激活并整体转发。
        harness.reader.pump().expect("读取成功");
        assert!(harness.reader.shared.active.load(Ordering::Acquire));
        let block = harness.fsm.raw_rx.recv().expect("标记块已入通道");
        assert_eq!(block, full);
    }

    #[test]
    fn probe_zrqinit_answers_zrinit() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let sent = harness.wire();
        assert_eq!(sent, build_hex_header(ZRINIT, [0, 0, 0, receiver_flags()]));
        assert!(matches!(harness.fsm.mode, Mode::Deciding));
    }

    #[test]
    fn probe_zrinit_poll_triggers_await_upload() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        // lrzsz rz 轮询头：** ^X B 01 00 00 00 23 + CRC(be50)。
        harness
            .fsm
            .consume_block(b"**\x18B0100000023be50\r".to_vec());
        assert!(matches!(harness.fsm.mode, Mode::AwaitUpload));
        assert!(matches!(
            harness.collect_events().first(),
            Some(ZmodemEvent::AwaitUpload)
        ));
    }

    #[test]
    fn full_sz_download_session_lands_file() {
        let dir = std::env::temp_dir().join(format!("transom-zmodem-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录创建成功");
        let mut harness = harness(dir.clone(), true);
        // 1) sz 的 ZRQINIT → 应答 ZRINIT。
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness.fsm.mode = Mode::Download;
        // 2) ZSINIT 头 + Attn 子包 → 应答 ZACK(1)。
        let zsinit = build_bin16_header(ZSINIT, [0, 0, 0, 0]);
        let attn = build_subpacket16(b"\x18\x18", ZCRCW);
        harness.fsm.consume_block([zsinit, attn].concat());
        let sent = harness.wire();
        assert!(
            sent.ends_with(&build_hex_header(3, [1, 0, 0, 0])),
            "ZSINIT 应答 ZACK 位置 1"
        );
        // 3) ZFILE 头 + 文件信息子包 → 创建文件并应答 ZRPOS(0)。
        let zfile = build_bin16_header(ZFILE, [0, 0, 0, 0]);
        let info = build_subpacket16(b"hello.txt 5 0 644 1 5", ZCRCW);
        harness.fsm.consume_block([zfile, info].concat());
        let sent = harness.wire();
        assert!(sent.ends_with(&build_hex_header(ZRPOS, 0u32.to_le_bytes())));
        // 4) ZDATA（bin32/CRC-32 子包）+ ZEOF → 写盘并应答 ZRINIT。
        let zdata = build_bin32_header(10, [0, 0, 0, 0]);
        let first = build_subpacket32(b"hello", ZCRCG);
        let last = build_subpacket32(b"", ZCRCE);
        let zeof = build_bin32_header(ZEOF, 5u32.to_le_bytes());
        harness
            .fsm
            .consume_block([zdata, first, last, zeof].concat());
        let sent = harness.wire();
        assert!(sent.ends_with(&build_hex_header(ZRINIT, [0, 0, 0, receiver_flags()])));
        // 5) 对端 ZFIN → 应答 ZFIN，随后 "OO" 收尾。
        harness
            .fsm
            .consume_block(build_bin16_header(ZFIN, [0, 0, 0, 0]));
        assert!(
            harness
                .wire()
                .ends_with(&build_hex_header(ZFIN, [0, 0, 0, 0]))
        );
        harness.fsm.consume_block(b"OO".to_vec());
        assert!(matches!(harness.fsm.mode, Mode::Idle));

        let content = std::fs::read(dir.join("hello.txt")).expect("下载文件已落盘");
        assert_eq!(content, b"hello");
        std::fs::remove_file(dir.join("hello.txt")).expect("清理");
        std::fs::remove_dir(&dir).expect("清理");

        let events = harness.collect_events();
        assert!(events.contains(&ZmodemEvent::Started {
            direction: ZmodemDirection::Download
        }));
        assert!(
            events.contains(&ZmodemEvent::Finished(ZmodemFinished::Completed {
                files: 1
            }))
        );
    }

    #[test]
    fn upload_decision_with_missing_file_aborts() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B0100000023be50\r".to_vec());
        let _ = harness.wire();
        assert!(harness.ctrl(ZmodemCommand::Upload(vec![PathBuf::from(
            "/nonexistent.bin"
        )])));
        // 文件打不开且无后续文件：取消序列 + 失败收尾。
        let sent = harness.wire();
        assert!(sent.starts_with(&[ZDLE; 8][..]));
        assert!(matches!(harness.fsm.mode, Mode::Idle,));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
    }

    #[test]
    fn cancel_during_download_writes_can_sequence() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness.fsm.mode = Mode::Download;
        assert!(harness.ctrl(ZmodemCommand::Cancel));
        assert!(harness.wire().starts_with(&[ZDLE; 8][..]));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Cancelled))
        ));
    }

    #[test]
    fn find_mark_matches_hex_and_bin32_only() {
        assert_eq!(find_mark(b"**\x18B01"), Some(0));
        assert_eq!(find_mark(b"x**\x18C0100"), Some(1));
        assert_eq!(find_mark(b"**\x18A0100"), None, "非 hex/bin32 格式不触发");
        assert_eq!(find_mark(b"*\x18B01"), None);
    }

    /// 下载首帧前必须把缺失的落盘目录建出来：否则每个 ZFILE 建文件都失败，
    /// 会话会退化成逐文件 ZSKIP 并报"完成 0 个文件"。
    #[test]
    fn download_creates_missing_directory_before_answering() {
        let base =
            std::env::temp_dir().join(format!("transom-zmodem-mkdir-{}", std::process::id()));
        let nested = base.join("nested/sub");
        let mut harness = harness(nested.clone(), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        let zfile = build_bin16_header(ZFILE, [0, 0, 0, 0]);
        let info = build_subpacket16(b"landed.txt 3 0 644 1 3", ZCRCW);
        harness.fsm.consume_block([zfile, info].concat());
        assert!(matches!(harness.fsm.mode, Mode::Download));
        assert!(
            nested.join("landed.txt").is_file(),
            "多级缺失目录应在 ZFILE 应答前创建"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 落盘目录不可创建（路径被普通文件占用）时整场会话无望：直接取消并给出
    /// 失败结局，而不是让对端误判为传输成功。
    #[test]
    fn download_aborts_when_directory_unusable() {
        let blocker =
            std::env::temp_dir().join(format!("transom-zmodem-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"x").expect("阻塞文件写入成功");
        let mut harness = harness(blocker.join("sub"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness
            .fsm
            .consume_block(build_bin16_header(ZFILE, [0, 0, 0, 0]));
        let sent = harness.wire();
        assert!(
            sent.starts_with(&[ZDLE; 8][..]),
            "目录不可建时先发出取消序列"
        );
        assert!(matches!(harness.fsm.mode, Mode::Idle));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
        std::fs::remove_file(&blocker).expect("清理阻塞文件");
    }

    /// 驱动线程退出后过滤层必须停止接管字节流：否则协议通道无人消费，
    /// 终端表现为输出冻结且键入全部丢失。
    #[test]
    fn lifeline_drop_restores_passthrough() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.reader.shared.active.store(true, Ordering::Release);
        drop(FsmLifeline {
            shared: Arc::clone(&harness.reader.shared),
            events: harness.fsm.events.clone(),
        });
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert!(harness.reader.shared.terminated.load(Ordering::Acquire));
        harness.reader.inner = Box::new(Cursor::new(b"**\x18B00000000000000\r".to_vec()));
        let mut out = [0u8; 64];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(
            &out[..count],
            b"**\x18B00000000000000\r",
            "线程终止后协议字节应原样回到终端通路"
        );
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
    }

    /// panic 展开同样要复位共享状态并上报失败结局，界面才能清除"传输中"徽章。
    #[test]
    fn lifeline_reports_failure_when_thread_panics() {
        let (event_tx, event_rx) = flume::bounded::<ZmodemEvent>(4);
        let shared = Arc::new(Shared {
            active: AtomicBool::new(true),
            terminated: AtomicBool::new(false),
        });
        let shared_for_thread = Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _lifeline = FsmLifeline {
                shared: shared_for_thread,
                events: event_tx,
            };
            panic!("测试用驱动线程故障");
        });
        assert!(handle.join().is_err(), "子线程应以 panic 结束");
        assert!(
            !shared.active.load(Ordering::Acquire),
            "panic 展开须复位激活标志"
        );
        assert!(shared.terminated.load(Ordering::Acquire));
        let event = event_rx.recv().expect("panic 展开应上报失败结局");
        assert!(matches!(
            event,
            ZmodemEvent::Finished(ZmodemFinished::Failed(_))
        ));
    }

    /// 决策窗口过期之后才提交的上传必须回报可见失败：否则用户的观感只是
    /// "文件对话框关掉、什么都没发生"。
    #[test]
    fn upload_outside_wait_window_reports_failure_to_ui() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        assert!(harness.ctrl(ZmodemCommand::Upload(vec![PathBuf::from("/tmp/a")])));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
        assert!(
            matches!(harness.fsm.mode, Mode::Idle),
            "过期决策不应改变会话阶段"
        );
    }

    /// 连接断开必须给在途会话一个结局：否则徽章停在"传输中"，Esc 也会发给
    /// 一个已经不存在的会话。
    #[test]
    fn disconnect_reports_outcome_for_in_flight_session() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.fsm.started = true;
        harness.fsm.mode = Mode::Download;
        harness.reader.shared.active.store(true, Ordering::Release);
        harness.fsm.abort_in_flight();
        assert!(matches!(harness.fsm.mode, Mode::Idle));
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
    }

    /// 从未开始的会话不需要编造结局：无徽章即无上报。
    #[test]
    fn disconnect_without_session_stays_silent() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.fsm.abort_in_flight();
        assert!(harness.collect_events().is_empty());
    }
}
