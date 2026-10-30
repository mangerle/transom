use alacritty_terminal::event::{Event, EventListener};
use flume::Sender;

/// 命令历史候选的查询回调：按输入前缀返回若干条历史命令。
///
/// 用回调而不是把存储塞进视图：终端 crate 不该知道历史存在哪个文件、留多少条，
/// 那些都是应用侧的策略。回调在 UI 线程同步调用，实现方必须走内存快照。
pub type HistoryProvider = std::sync::Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// Events surfaced to the application view.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// 终端程序通过 OSC 0/2 设置的窗口标题；空串表示显式清空。
    Title(String),
    /// 标题栈弹出或终端要求恢复默认标题：运行期标题应回落。
    ResetTitle,
    /// 终端程序通过 OSC 7 上报的当前工作目录（已解码为裸路径）。
    Cwd(String),
    Bell,
    Exit,
    ClipboardStored,
    /// 远端程序请求写入本地剪贴板，尚未获得本会话授权。
    ///
    /// 宿主收到后必须弹一次显式确认；用户在确认框里允许后，本会话内的后续
    /// 写入直接生效，不再重复询问。拒绝只作用于当前这一次请求。
    ClipboardWriteRequest {
        text: String,
    },
    /// Shell 集成报告一条命令跑完了：退出码可能为 `None`（老集成只发提示符标记），
    /// 耗时由集成上报或按本地计时测得，单位毫秒。
    CommandFinished {
        exit_code: Option<i32>,
        duration_ms: u64,
    },
    /// 采集到一条用户执行过的命令文本，供历史与建议使用。
    ///
    /// 只在能确定命令内容时才发：shell 集成上报的参数或提示符到光标之间的网格
    /// 文本，二者都取不到就不发——猜出来的历史比没有历史更糟。
    CommandRecorded {
        command: String,
    },
    /// 终端程序通过 OSC 777 或 OSC 9 显式请求的桌面通知。
    DesktopNotification {
        title: String,
        body: String,
    },
    /// The user asked for a context menu at a window-local position.
    ContextMenuRequested {
        x: f32,
        y: f32,
    },
    /// 用户把外部文件拖到终端网格上并松手。
    ///
    /// 终端视图只负责"命中区域 = 网格"与事件投递：文件要不要接、传到哪台
    /// 主机的哪个目录，取决于会话类型与远端 cwd，属于应用层的策略。
    FilesDropped(Vec<std::path::PathBuf>),
    /// 终端输出流命中预设触发器规则。
    TriggerFired {
        rule_name: String,
        action: transom_core::TriggerAction,
    },
    /// 用户点击了 OSC 8 代码位置链接（`file:line:col`）。
    ///
    /// 终端只负责识别与投递：路径是远端还是本地、开哪个编辑器、相对路径
    /// 按哪个工作目录解析，全部由应用层决定。行列均为 1 基，缺列时为 `None`。
    OpenCodeLocation {
        path: String,
        line: u32,
        column: Option<u32>,
    },
}

/// Bridges alacritty terminal events into a bounded channel consumed by the view.
#[derive(Clone)]
pub struct TerminalEventProxy {
    sender: Sender<Event>,
}

impl TerminalEventProxy {
    pub fn new(sender: Sender<Event>) -> Self {
        Self { sender }
    }
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        // Wakeups fire for every output chunk; the reader loop already notifies.
        if matches!(event, Event::Wakeup) {
            return;
        }
        let _ = self.sender.try_send(event);
    }
}
