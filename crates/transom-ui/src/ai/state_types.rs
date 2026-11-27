//! AI 助手面板的基础状态枚举、会话摘要与聊天消息项类型定义。

use std::sync::Arc;

use transom_core::{HostProtection, SessionId, TabId};

use super::markdown_blocks::MarkdownSegment;

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
        /// 已定稿消息的预解析 Markdown 分段缓存（流式过程中为 `None`，避免每 25ms 帧全量重解析）。
        cached_segments: Option<Arc<Vec<MarkdownSegment>>>,
    },
    ToolCard {
        id: String,
        call_id: String,
        running: bool,
        tool_name: String,
        host_label: String,
        risk_label: String,
        summary: String,
        output: String,
        ok: bool,
        duration_ms: u64,
        diff_preview: Option<String>,
        revert_target_path: Option<String>,
        revert_backup_path: Option<String>,
    },
    /// 审批决策或权限变更的轻量化单行审计锚点。
    AuditNotice {
        id: String,
        approved: bool,
        label: String,
        host_label: String,
    },
}
