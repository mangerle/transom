//! AI 助手面板的基础状态枚举、会话摘要与聊天消息项类型定义。

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::RenderImage;
use transom_core::{HostProtection, SessionId, TabId};

use super::markdown_blocks::StreamingMarkdownCache;

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

/// 当前 AI 运行的显式身份与生命周期状态。
#[derive(Clone, Debug)]
pub(crate) struct ActiveAiRun {
    pub(crate) run_id: String,
    /// 新对话在收到 `RunStarted` 前尚不知道真实会话 ID。
    pub(crate) conversation_id: Option<String>,
    pub(crate) started_at: Instant,
    pub(crate) cancel_requested: bool,
    /// 用户已切换/新建会话而脱离原运行视图时置为 true，防止旧运行事件篡改新视图。
    pub(crate) detached: bool,
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

/// AI 主目标会话选择模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AiTargetMode {
    /// 默认跟随工作区当前激活的标签页。
    #[default]
    FollowActiveTab,
    /// 强制使用全局上下文，不绑定任何主机。
    GlobalOnly,
    /// 手动锁定到指定标签页对应的已连接会话。
    PinnedTab(TabId),
}

/// 底部控制条当前展开的浮层菜单。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AiActivePopup {
    #[default]
    None,
    TargetSession,
    Backend,
    Mode,
    Model,
    SessionStats,
    TokenUsage,
    ContextUsage,
}

/// 用户消息中携带的单张图片缩略图与原始 Base64 载荷。
#[derive(Clone, Debug)]
pub(crate) struct UiChatImage {
    pub(crate) base64: String,
    pub(crate) thumb: Option<Arc<RenderImage>>,
}

/// 消息流中的单条展示项。
#[derive(Clone, Debug)]
pub(crate) enum UiChatItem {
    UserText {
        id: String,
        text: String,
        images: Vec<UiChatImage>,
        timestamp: Option<chrono::DateTime<chrono::Utc>>,
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
        /// 增量 Markdown 分段缓存（流式过程中仅重解析未闭合尾部，定稿后直接复用）。
        cached_segments: Option<StreamingMarkdownCache>,
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
