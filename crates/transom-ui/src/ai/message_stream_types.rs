//! 消息流实体对外契约：向上冒泡的交互事件与向下注入的运行态快照。

use std::time::Instant;

use super::task_bar::UiManagedTask;

/// 消息流实体向父视图发出的交互意图事件。
#[derive(Debug, Clone)]
pub(in crate::app::ai) enum MessageStreamEvent {
    ToggleCard(String),
    RollbackCheckpoint(String),
    SendQuickPrompt(String),
    CopyText {
        key: String,
        text: String,
    },
    SendToTerminal(String),
    SaveSnippet {
        name: String,
        command: String,
    },
    RevertRemoteFile {
        host: String,
        target: String,
        backup: String,
    },
    JumpToTerminal(String),
    SubagentAction {
        action: String,
        task_id: String,
    },
    LoadOlderMessages,
    PreviewImage {
        title: String,
        base64: String,
    },
}

/// 由父视图单向注入的消息流运行态元信息快照（不含消息列表深拷贝）。
pub(in crate::app::ai) struct MessageStreamSnapshot {
    pub(in crate::app::ai) is_running: bool,
    pub(in crate::app::ai) run_started_at: Option<Instant>,
    pub(in crate::app::ai) active_focus: Option<String>,
    pub(in crate::app::ai) status_error: Option<String>,
    pub(in crate::app::ai) copied_key: Option<String>,
    pub(in crate::app::ai) has_active_terminal: bool,
    pub(in crate::app::ai) active_acp: Option<(String, String)>,
    pub(in crate::app::ai) managed_tasks: Vec<UiManagedTask>,
}
