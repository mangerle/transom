//! ZMODEM 状态机会话阶段与通道契约定义。

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use transom_core::ZmodemEvent;

use crate::attach::Shared;

/// ZRQINIT 之后的分流等待窗口：sz 的 ZSINIT/ZFILE 紧随其后，静默则视为
/// 非 lrzsz 的 rz 变体（直接等 ZFILE），转上传决策。
pub(crate) const DECIDE_TIMEOUT: Duration = Duration::from_secs(2);
/// 标记已激活过滤层后等待首个合法帧的上限。
///
/// 终端输出里的 `**` + ZDLE 可能只是二进制噪声的巧合；没有这个上限，
/// 状态机会在待命期无限阻塞，过滤层持续吞掉输出与键入，只能断线恢复。
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
/// 传输中的字节级静默上限：超过即视为对端假死。
pub(crate) const TRANSFER_TIMEOUT: Duration = Duration::from_secs(15);
/// 等待用户选择上传文件的窗口；超时放弃，对端 rz 轮询失败自行退出。
///
/// 界面用阻塞式系统文件对话框收集决策，用户可能在目录树里翻找，窗口必须
/// 按"人手速"而不是"网络往返"量级取值；过期后仍会收到显式失败提示。
pub(crate) const UPLOAD_WAIT: Duration = Duration::from_secs(300);

/// 界面侧提交给状态机的控制命令。
pub(crate) enum ZmodemCommand {
    Upload(Vec<PathBuf>),
    Cancel,
}

/// 状态机与过滤层之间的全部通道与共享件，一次装配避免长参数列表。
pub(crate) struct FsmChannels {
    /// 真实线路写出端（与写过滤层共享）。
    pub(crate) writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// 事件上报通道。
    pub(crate) events: flume::Sender<ZmodemEvent>,
    /// 会话结束后回注终端的残余字节通道。
    pub(crate) replay_tx: flume::Sender<Vec<u8>>,
    /// 读过滤层转入的协议原始字节。
    pub(crate) raw_rx: flume::Receiver<Vec<u8>>,
    /// 界面决策命令。
    pub(crate) ctrl_rx: flume::Receiver<ZmodemCommand>,
    /// 过滤层激活状态。
    pub(crate) shared: Arc<Shared>,
}

/// 驱动线程的会话阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// 等待首块（对端发起标记已把过滤层激活）。
    Idle,
    /// 已见 ZRQINIT，等待 sz 的 ZSINIT/ZFILE 或超时转上传。
    Deciding,
    /// sz 下载中。
    Download,
    /// 等待用户选择上传文件。
    AwaitUpload,
    /// rz 上传中（含打开文件、等 ZRPOS、发数据、收尾全流程）。
    Upload,
}
