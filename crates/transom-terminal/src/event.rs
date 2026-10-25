use alacritty_terminal::event::{Event, EventListener};
use flume::Sender;

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
