//! 应用级模态弹窗状态，按领域分组。
//!
//! 设计原理（Why）：这些弹窗的开关入口分散在菜单、快捷键、拖拽与异步回调，
//! 短期内无法全部搬进各自实体；但把十几个互不相关的字段平铺在同一个结构里，
//! 读者无法回答“谁能改这个弹窗”。按会话、连接、工具三组划分后，每组只包含
//! 同一生命周期的弹窗，新增弹窗必须先判断它属于哪一组。
//!
//! # 代价与局限
//! 分组只解决可读性与归属提示，不改变“外壳统一持有”的事实；设置弹窗已经
//! 独立为实体，后续应把会话表单、连接确认也按同一路径迁成实体，最终让本
//! 容器退化为“实体句柄集合”。

use super::batch::BatchEditForm;
use crate::backup::BackupPromptState;
use crate::connection::{AuthPromptState, HostKeyConfirmState, PendingCredential};
use crate::keys::KnownHostsModalState;
use crate::session_form::SessionForm;
use crate::settings::SettingsView;
use crate::sidebar::FolderForm;
use crate::snippets::SnippetPanelState;
use gpui_kit::Entity;
use gpui_kit::component::command::CommandState;

/// 会话与文件夹表单类弹窗。
pub(crate) struct SessionModals {
    pub(crate) form: Option<SessionForm>,
    pub(crate) folder_form: Option<FolderForm>,
    pub(crate) batch_edit: Option<BatchEditForm>,
}

impl SessionModals {
    fn new() -> Self {
        Self {
            form: None,
            folder_form: None,
            batch_edit: None,
        }
    }
}

/// 连接建立过程中的确认与授权弹窗。
pub(crate) struct ConnectionModals {
    pub(crate) auth_prompt: Option<AuthPromptState>,
    pub(crate) host_key_confirm: Option<HostKeyConfirmState>,
    /// 远端 OSC 52 写剪贴板的待裁决请求（默认拒绝，按会话授权）。
    pub(crate) clipboard_request: Option<crate::clipboard_prompt::ClipboardWritePrompt>,
    pub(crate) pending_credential: Option<PendingCredential>,
    pub(crate) known_hosts: KnownHostsModalState,
}

/// 工具类浮层：查看器、备份、快捷入口与一次性确认。
pub(crate) struct ToolModals {
    pub(crate) replay: Option<crate::record::ReplayModalState>,
    pub(crate) audit_viewer: Option<crate::audit_viewer::AuditViewer>,
    pub(crate) backup_prompt: Option<BackupPromptState>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    pub(crate) snippet_panel: SnippetPanelState,
    pub(crate) confirm_forward_any: Option<crate::forward::PendingForwardConfirm>,
    /// 密钥生成与管理向导；未打开时为空。
    pub(crate) key_wizard: Option<crate::keys::KeyWizardState>,
}

/// Git 仓库管理、克隆与分支操作类弹窗。
pub(crate) struct GitModals {
    pub(crate) project: Option<crate::git::GitProjectModal>,
    pub(crate) clone: Option<crate::git::GitCloneModal>,
    pub(crate) worktree: Option<crate::git::GitWorktreeModal>,
    pub(crate) name: Option<crate::git::GitNameModal>,
    pub(crate) rebase: Option<crate::git::GitRebaseModal>,
    pub(crate) log: Option<transom_core::TabId>,
}

impl GitModals {
    fn new() -> Self {
        Self {
            project: None,
            clone: None,
            worktree: None,
            name: None,
            rebase: None,
            log: None,
        }
    }
}

/// 聚合应用内全部顶层交互弹窗与表单状态。
pub(crate) struct ModalHub {
    pub(crate) session: SessionModals,
    pub(crate) connection: ConnectionModals,
    pub(crate) tools: ToolModals,
    pub(crate) git: GitModals,
    /// 设置弹窗已经实体化，这里只保留句柄。
    pub(crate) settings: Entity<SettingsView>,
}

impl ModalHub {
    /// 关闭不承载用户输入的轻量浮层（快捷打开与快捷命令卡片）。
    ///
    /// 承载表单或确认的模态不在此列：进入沉浸模式不等于放弃未提交的编辑。
    /// 返回是否有浮层被关闭，调用方据此决定是否需要重绘。
    pub(crate) fn close_transient(&mut self) -> bool {
        let quick_open = std::mem::replace(&mut self.tools.show_quick_open, false);
        let snippet = std::mem::replace(&mut self.tools.snippet_panel.open, false);
        quick_open || snippet
    }

    /// 构造新的模态聚合容器。
    pub(crate) fn new(
        known_hosts: KnownHostsModalState,
        settings: Entity<SettingsView>,
        quick_open_state: Entity<CommandState>,
        snippet_panel: SnippetPanelState,
    ) -> Self {
        Self {
            session: SessionModals::new(),
            connection: ConnectionModals {
                auth_prompt: None,
                host_key_confirm: None,
                clipboard_request: None,
                pending_credential: None,
                known_hosts,
            },
            tools: ToolModals {
                replay: None,
                audit_viewer: None,
                backup_prompt: None,
                show_quick_open: false,
                quick_open_state,
                snippet_panel,
                confirm_forward_any: None,
                key_wizard: None,
            },
            git: GitModals::new(),
            settings,
        }
    }
}
