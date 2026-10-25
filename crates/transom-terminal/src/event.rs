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
    Title(String),
    ResetTitle,
    Bell,
    Exit,
    ClipboardStored,
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
    /// 终端程序通过 OSC 777 显式请求的桌面通知。
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
