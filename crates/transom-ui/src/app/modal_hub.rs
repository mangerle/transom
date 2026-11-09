//! 应用级模态弹窗状态，按领域分组。
//!
//! 设计原理（Why）：这些弹窗的开关入口分散在菜单、快捷键、拖拽与异步回调，
//! 短期内无法全部搬进各自实体；但把十几个互不相关的字段平铺在同一个结构里，
//! 读者无法回答“谁能改这个弹窗”。按会话、连接、工具三组划分后，每组只包含
//! 同一生命周期的弹窗，新增弹窗必须先判断它属于哪一组。
//!
//! # 代价与局限
//! 分组只解决可读性与归属提示，不改变“外壳统一持有”的事实；设置、已知主机、
//! 备份、审计查看器、密钥向导、会话回放、会话表单与连接确认弹窗已经独立为实体，
//! 本容器正逐步退化为“实体句柄集合”。

use super::batch::BatchEditForm;
use crate::audit_viewer::AuditViewerView;
use crate::backup::BackupPromptView;
use crate::connection::{ConnectionPromptsView, PendingCredential};
use crate::container::ContainerFormView;
use crate::keys::{KeyWizardView, KnownHostsView};
use crate::record::ReplayModalView;
use crate::session_form::SessionFormView;
use crate::settings::SettingsView;
use crate::sidebar::FolderForm;
use crate::snippets::SnippetPanelState;
use gpui_kit::Entity;
use gpui_kit::component::command::CommandState;

/// 会话与文件夹表单类弹窗。
pub(crate) struct SessionModals {
    /// 会话新建/编辑表单；打开状态、输入控件与表单动作都归实体。
    pub(crate) form: Entity<SessionFormView>,
    pub(crate) folder_form: Option<FolderForm>,
    pub(crate) batch_edit: Option<BatchEditForm>,
}

impl SessionModals {
    fn new(form: Entity<SessionFormView>) -> Self {
        Self {
            form,
            folder_form: None,
            batch_edit: None,
        }
    }
}

/// 连接建立过程中的确认与授权弹窗。
pub(crate) struct ConnectionModals {
    /// SSH 鉴权、首连指纹与 OSC 52 授权三个窗口级弹窗；打开状态与应答通道归实体。
    pub(crate) prompts: Entity<ConnectionPromptsView>,
    pub(crate) pending_credential: Option<PendingCredential>,
    pub(crate) known_hosts: Entity<KnownHostsView>,
}

/// 工具类浮层中需要由外壳预创建的实体句柄。
pub(crate) struct ToolModalEntities {
    pub(crate) backup_prompt: Entity<BackupPromptView>,
    /// 审计日志查看器；打开状态与在途扫描归实体。
    pub(crate) audit_viewer: Entity<AuditViewerView>,
    /// 密钥生成与管理向导；打开状态、公钥清单与在途生成任务归实体。
    pub(crate) key_wizard: Entity<KeyWizardView>,
    /// 会话录制回放选择弹窗；打开状态、脚本清单与在途扫描归实体。
    pub(crate) replay: Entity<ReplayModalView>,
    /// 容器引擎连接表单；打开状态、输入控件与测试连接动作归实体。
    pub(crate) container_form: Entity<ContainerFormView>,
}

/// 工具类浮层：查看器、备份、快捷入口与一次性确认。
pub(crate) struct ToolModals {
    /// 会话录制回放选择弹窗实体；打开状态与在途扫描都归实体。
    pub(crate) replay: Entity<ReplayModalView>,
    /// 审计日志查看器实体；打开状态与在途扫描都归实体。
    pub(crate) audit_viewer: Entity<AuditViewerView>,
    /// 备份导入/导出弹窗实体；打开状态与在途任务都归实体。
    pub(crate) backup_prompt: Entity<BackupPromptView>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    pub(crate) snippet_panel: SnippetPanelState,
    pub(crate) confirm_forward_any: Option<crate::forward::PendingForwardConfirm>,
    /// 密钥生成与管理向导实体；打开状态由实体持有。
    pub(crate) key_wizard: Entity<KeyWizardView>,
    /// 容器引擎连接表单实体；打开状态、输入控件与测试连接动作归实体。
    pub(crate) container_form: Entity<ContainerFormView>,
}

/// Git 仓库管理、克隆与分支操作类弹窗。
pub(crate) struct GitModals {
    pub(crate) project: Option<crate::git::GitProjectModal>,
    pub(crate) clone: Option<crate::git::GitCloneModal>,
    pub(crate) worktree: Option<crate::git::GitWorktreeModal>,
    pub(crate) name: Option<crate::git::GitNameModal>,
    pub(crate) rebase: Option<crate::git::GitRebaseModal>,
    pub(crate) push: Option<crate::git::GitPushModal>,
    pub(crate) conflict: Option<crate::git::GitConflictModal>,
}

impl GitModals {
    fn new() -> Self {
        Self {
            project: None,
            clone: None,
            worktree: None,
            name: None,
            rebase: None,
            push: None,
            conflict: None,
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
        session_form: Entity<SessionFormView>,
        known_hosts: Entity<KnownHostsView>,
        connection_prompts: Entity<ConnectionPromptsView>,
        settings: Entity<SettingsView>,
        tools: ToolModalEntities,
        quick_open_state: Entity<CommandState>,
        snippet_panel: SnippetPanelState,
    ) -> Self {
        Self {
            session: SessionModals::new(session_form),
            connection: ConnectionModals {
                prompts: connection_prompts,
                pending_credential: None,
                known_hosts,
            },
            tools: ToolModals {
                replay: tools.replay,
                audit_viewer: tools.audit_viewer,
                backup_prompt: tools.backup_prompt,
                show_quick_open: false,
                quick_open_state,
                snippet_panel,
                confirm_forward_any: None,
                key_wizard: tools.key_wizard,
                container_form: tools.container_form,
            },
            git: GitModals::new(),
            settings,
        }
    }
}
