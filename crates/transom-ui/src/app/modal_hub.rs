//! 模态弹窗与浮层状态聚合容器。
//!
//! 设计说明（Why）：
//! - 实现初衷：从上帝对象 `AppShell` 中剥离 10 余种弹窗与交互表单的松散状态，
//!   消除对顶层外壳字段平铺的污染；
//! - 核心优势：
//!   1. 集中管理全部弹窗的显隐与表单输入生命周期；
//!   2. 统一提供重置与活动弹窗判定能力；
//!   3. 大幅精简 `AppShell` 结构体，提高领域内聚度。

use crate::backup::BackupPromptState;
use crate::batch::BatchEditForm;
use crate::connection::{AuthPromptState, HostKeyConfirmState, PendingCredential};
use crate::keys::KnownHostsModalState;
use crate::session_form::SessionForm;
use crate::settings::SettingsModalState;
use crate::sidebar::FolderForm;
use crate::snippets::SnippetPanelState;
use gpui_kit::Entity;
use gpui_kit::component::command::CommandState;

/// 聚合应用内全部顶层交互弹窗与表单状态。
pub(crate) struct ModalHub {
    pub(crate) form: Option<SessionForm>,
    pub(crate) folder_form: Option<FolderForm>,
    pub(crate) batch_edit: Option<BatchEditForm>,
    pub(crate) auth_prompt: Option<AuthPromptState>,
    pub(crate) host_key_confirm: Option<HostKeyConfirmState>,
    /// 远端 OSC 52 写剪贴板的待裁决请求（默认拒绝，按会话授权）。
    pub(crate) clipboard_request: Option<crate::clipboard_prompt::ClipboardWritePrompt>,
    pub(crate) pending_credential: Option<PendingCredential>,
    pub(crate) known_hosts: KnownHostsModalState,
    pub(crate) settings: SettingsModalState,
    pub(crate) replay: Option<crate::record::ReplayModalState>,
    pub(crate) audit_viewer: Option<crate::audit_viewer::AuditViewer>,
    pub(crate) backup_prompt: Option<BackupPromptState>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    pub(crate) snippet_panel: SnippetPanelState,
    pub(crate) confirm_forward_any: Option<crate::forward::PendingForwardConfirm>,
}

impl ModalHub {
    /// 关闭不承载用户输入的轻量浮层（快捷打开与快捷命令卡片）。
    ///
    /// 承载表单或确认的模态不在此列：进入沉浸模式不等于放弃未提交的编辑。
    /// 返回是否有浮层被关闭，调用方据此决定是否需要重绘。
    pub(crate) fn close_transient(&mut self) -> bool {
        let quick_open = std::mem::replace(&mut self.show_quick_open, false);
        let snippet = std::mem::replace(&mut self.snippet_panel.open, false);
        quick_open || snippet
    }

    /// 构造新的模态聚合容器。
    pub(crate) fn new(
        known_hosts: KnownHostsModalState,
        settings: SettingsModalState,
        quick_open_state: Entity<CommandState>,
        snippet_panel: SnippetPanelState,
    ) -> Self {
        Self {
            form: None,
            folder_form: None,
            batch_edit: None,
            auth_prompt: None,
            host_key_confirm: None,
            clipboard_request: None,
            pending_credential: None,
            known_hosts,
            settings,
            replay: None,
            audit_viewer: None,
            backup_prompt: None,
            show_quick_open: false,
            quick_open_state,
            snippet_panel,
            confirm_forward_any: None,
        }
    }
}
