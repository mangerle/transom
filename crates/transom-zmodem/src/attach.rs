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
///
/// 与终端视图的读取缓冲对齐到 64 KiB：透传路径每次上层 read 恰好消费一整
/// 块，若这里比上层小，上层缓冲再大也只能按本层块粒度逐个搬运，读块的
/// 收益会被本层吃掉。
const READ_CHUNK: usize = 64 * 1024;
/// 发起标记长度：`*` `*` ZDLE 格式字节。
const MARK_LEN: usize = 4;
/// 转发块的等待窗口：驱动线程短暂无响应时重查会话状态，避免读线程
/// 因无人消费而永久卡死。
const FORWARD_WAIT: Duration = Duration::from_secs(1);
/// 原始字节通道容量（块粒度）：64 KiB × 32 ≈ 2 MiB 上限，与原
/// 8 KiB × 256 同量级，块增大不抬高内存上限；打满时读线程阻塞形成背压。
const RAW_CAPACITY: usize = 32;
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
///
/// 先定位 `*` 再校验后续三字节：空闲透传时绝大多数字节只做一次比较，
/// 而不是原始实现的固定四次比较。终端高吞吐输出（10 MB/s 量级、每秒
/// 上千万字节）下这是读路径的主要扫描成本，此改动把常数从 4 降到 ~1。
fn find_mark(data: &[u8]) -> Option<usize> {
    let last_start = data.len().checked_sub(MARK_LEN)?;
    let mut from = 0;
    while from <= last_start {
        let offset = data[from..=last_start]
            .iter()
            .position(|&byte| byte == b'*')?;
        let start = from + offset;
        let window = &data[start..start + MARK_LEN];
        if window[1] == b'*' && window[2] == ZDLE && (window[3] == ZHEX || window[3] == ZBIN32) {
            return Some(start);
        }
        from = start + 1;
    }
    None
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
        integration_preinstalled,
        cleanup,
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
            // 会话级标记与清理守卫必须原样带走：重组 io 不能让预装状态丢失，
            // 也不能让临时资源提前释放。
            integration_preinstalled,
            cleanup,
        },
        ZmodemControl { tx: ctrl_tx },
    )
}

#[cfg(test)]
mod tests {
    include!("attach_tests.rs");
}
