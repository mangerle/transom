//! AI 助手面板的基础状态枚举、会话摘要与聊天消息项类型定义。

use transom_core::{HostProtection, SessionId, TabId};

/// 外壳单向注入的活动会话摘要（无会话时为 `None`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveAiSessionInfo {
    pub(crate) tab_id: TabId,
    pub(crate) session_id: SessionId,
    pub(crate) host_name: String,
    pub(crate) login: String,
    pub(crate) platform: String,
    pub(crate) protection: HostProtection,
    pub(crate) connected: bool,
}

/// 面板右侧抽屉视图模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AiDrawerTab {
    #[default]
    Chat,
    History,
    Config,
    Memory,
    Skills,
}

/// 设置抽屉顶部分区标签。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AiConfigSection {
    #[default]
    Models,
    General,
    PolicyHooks,
}

/// 底部控制条当前展开的浮层菜单。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AiActivePopup {
    #[default]
    None,
    Backend,
    Mode,
    Model,
    SessionStats,
    TokenUsage,
    ContextUsage,
}

/// 消息流中的单条展示项。
#[derive(Clone, Debug)]
pub(crate) enum UiChatItem {
    UserText {
        id: String,
        text: String,
        timestamp: Option<String>,
    },
    Reasoning {
        id: String,
        text: String,
        streaming: bool,
    },
    AssistantText {
        id: String,
        text: String,
        model: Option<String>,
        tokens_per_sec: Option<f32>,
        streaming: bool,
    },
    ToolCard {
        id: String,
        tool_name: String,
        host_label: String,
        risk_label: String,
        summary: String,
        output: String,
        ok: bool,
        duration_ms: u64,
        diff_preview: Option<String>,
    },
}
