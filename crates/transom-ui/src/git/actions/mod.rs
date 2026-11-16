//! Git 面板的动作派发与事件落地的公共层：项目列表、标签页生命周期与状态登记。
//!
//! 设计原理（Why）：
//! - 界面只发命令、只收事件，绝不直接跑子进程；所有方法都是「改状态 + 发命令」
//!   两步，失败路径统一由 `ReadFailed` / `ActionDone` 事件带回，避免在每个
//!   点击回调里各写一遍错误处理；
//! - 事件按 `project_id` 落到对应标签页：同一仓库可能在两个窗口/两个标签里
//!   同时打开，逐个标签匹配而不是记住「最后一次请求属于哪个标签」，才不会
//!   在快速切换时把 A 仓库的日志写进 B 仓库的视图；
//! - 写动作成功后立刻自动刷新：git 的索引是外部可变的（用户可能在命令行里
//!   同时操作），让界面停留在动作前的快照等于显示假数据；
//! - 具体动作按对象分组拆成同级子模块（`log_actions` / `ref_actions` /
//!   `shelf_actions` / `event_dispatch`），本文件只留项目与标签这一公共层，
//!   并对外再导出各模块的类型：调用方的引用路径不因拆分而改变。

use super::GitWorkspace;
use super::entity::GitWorkspaceEvent;
use super::form::GitProjectDraft;
use super::types::{GitBadge, GitWorkspaceState};
use crate::UiServices;
use crate::app::{
    AppShell, GitModals, PendingNotices, Persistence, TaskHub, TerminalTab, WindowRequests,
};
use crate::git::GitSidebarView;
use crate::palette::localized_error;
use crate::sidebar::{SidebarSection, SidebarState};
use crate::workspace::WorkspaceState;
use gpui_kit::*;
use transom_core::{
    Connection, GitCommand, GitProject, GitProjectId, GitRequestId, LocalConnection, Session, TabId,
};

mod commit_actions;
mod diff_actions;
pub(crate) use event_dispatch::dispatch_git_event;
mod diff_nav;
pub(crate) use diff_nav::DiffTabCloseScope;
mod diff_search;
mod event_dispatch;
mod init_event;
mod log_actions;
pub(crate) mod modal_bridges;
mod shelf_actions;
pub(crate) use shelf_actions::CreateWorktreeParams;
pub(crate) use sidebar_bridge::on_git_sidebar_event;
mod sidebar_actions;
mod sidebar_bridge;
mod workspace_refs;

// 统一收口对外导出的动作与事件分发自由函数，供 Git 各视图与主壳层按 `crate::git::actions::*` 调用。
pub(crate) use super::types::{GitDiffRequest, first_worktree_diff};

/// 历史改写与整合动作（界面意图，落到具体命令）。
pub(crate) enum GitHistoryAction {
    Reset {
        target: String,
        kind: transom_core::GitResetKind,
    },
    Revert(String),
    CherryPick(String),
    Merge(String),
    Rebase(String),
    RebaseAbort,
}

/// 项目与分组一次落盘。
///
/// 设计原理（Why）：两者必须同一次写完——分开保存会出现「分组已删、
/// 项目的 `folder_id` 还指着已消失分组」这种撕裂状态（同 `SessionStore` 的理由）。
/// Git 项目、标签与全局注册表动作的窄上下文。
///
/// 设计原理（Why）：Git 的跨域动作确实需要协调注册表、标签、命令、落盘与
/// 弹窗，但不需要整份应用状态；这里逐字段借用，调用方看不出它还能碰什么。
pub(crate) struct GitActions<'a> {
    /// Git 全局注册表实体：读写都经 `read`/`update`。
    sidebar: Entity<GitSidebarView>,
    /// 会话侧栏：只用于切换 Git 分组的展开态。
    session_sidebar: &'a mut SidebarState,
    /// 订阅表与后台任务表。
    tasks: &'a mut TaskHub,
    /// 命令通道、Git 项目存储等只读服务。
    services: &'a UiServices,
    /// 设置落盘入口。
    persistence: &'a Persistence,
    /// 标签页只读快照：标签创建、查找与应用状态。
    workspace: &'a mut WorkspaceState,
    /// Git 弹窗状态：供模态桥接从同一批字段构造。
    modals: &'a mut GitModals,
    /// 窗口级动作队列。
    window_requests: &'a mut WindowRequests,
    /// 命令队列满时的错误提示队列。
    pending_notices: &'a mut PendingNotices,
}

impl<'a> GitActions<'a> {
    /// 从应用状态拆借 Git 动作能力（字段级窄上下文）。
    pub(crate) fn new(state: &'a mut crate::app::AppState) -> Self {
        let crate::app::AppState {
            runtime,
            session,
            ui,
            workspace,
            features,
            ..
        } = state;
        Self {
            sidebar: features.git_sidebar.clone(),
            session_sidebar: &mut session.sidebar,
            tasks: &mut runtime.tasks,
            services: &runtime.services,
            persistence: &runtime.persistence,
            workspace,
            modals: &mut ui.modals.git,
            window_requests: &mut ui.window_requests,
            pending_notices: &mut ui.feedback.pending_notices,
        }
    }

    fn send_command(&mut self, command: impl Into<transom_core::AppCommand>) -> bool {
        match self.services.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                self.pending_notices
                    .push(crate::app::command_send_error_text(error));
                false
            }
        }
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.pending_notices.push(message);
    }

    fn set_transient_status(&mut self, message: impl Into<String>) {
        self.window_requests.push_transient_status(message);
    }

    /// 统一的设置落盘入口：失败记录日志并写入错误队列。
    fn persist_settings_reported(&mut self, settings: &transom_core::AppSettings) -> bool {
        match self.persistence.save_settings(settings) {
            Ok(()) => true,
            Err(error) => {
                log::error!("保存设置失败，界面仍是新值但下次启动会回退，原因: {error}");
                self.set_error(localized_error(&error));
                false
            }
        }
    }

    fn git_registry<'b>(&'b self, cx: &'b App) -> &'b super::types::GitHub {
        &self.sidebar.read(cx).state
    }

    fn update_git_registry<R>(
        &self,
        cx: &mut Context<AppShell>,
        update: impl FnOnce(&mut super::types::GitHub) -> R,
    ) -> R {
        self.sidebar.update(cx, |view, _| update(&mut view.state))
    }

    fn git_workspace(&self, tab_id: TabId) -> Option<Entity<GitWorkspace>> {
        self.workspace.tab(tab_id)?.git()
    }

    fn git_workspace_of_project(
        &self,
        project_id: GitProjectId,
        cx: &App,
    ) -> Option<Entity<GitWorkspace>> {
        self.workspace.tabs.iter().find_map(|tab| {
            let workspace = tab.git()?;
            (workspace.read(cx).state.project.id == project_id).then_some(workspace)
        })
    }
}

impl<'a> GitActions<'a> {
    pub(crate) fn persist_git_projects(&mut self, cx: &mut Context<AppShell>) {
        let registry = transom_core::GitRegistry {
            projects: self.git_registry(cx).projects.clone(),
            folders: self.git_registry(cx).folders.clone(),
        };
        if let Err(error) = self.services.git_projects.save(&registry) {
            self.set_error(localized_error(&error));
        }
        cx.notify();
    }

    /// 活动栏点击：切换左侧槽位的 Git 项目列表。
    pub(crate) fn toggle_git_sidebar(&mut self, cx: &mut Context<AppShell>) {
        let opening = self.session_sidebar.toggle_section(SidebarSection::Git);
        if opening {
            // 展开即刷一遍所有项目：列表里的分支与脏数必须是当下的。
            self.refresh_all_git_projects(cx);
        }
        cx.notify();
    }

    /// 刷新全部已登记项目的状态徽标。
    pub(crate) fn refresh_all_git_projects(&mut self, cx: &mut Context<AppShell>) {
        let projects = self.git_registry(cx).projects.clone();
        for project in projects {
            self.git_refresh(&project, cx);
        }
    }

    /// 刷新所有处于管理状态的 Git 项目状态（用于窗口重新聚焦时自动刷新）。
    pub(crate) fn git_refresh_active_projects(&mut self, cx: &mut Context<AppShell>) {
        self.refresh_all_git_projects(cx);
    }

    /// 采集单个项目的状态快照（侧栏徽标与标签页共用同一条命令）。
    pub(crate) fn git_refresh(&mut self, project: &GitProject, cx: &mut Context<AppShell>) {
        self.update_git_registry(cx, |git| {
            let badge = git.badges.entry(project.id).or_default();
            badge.loading = true;
            badge.error = None;
        });
        self.send_command(GitCommand::Refresh {
            project: project.clone(),
        });
        self.send_command(GitCommand::LoadRefs {
            project: project.clone(),
        });
        if let Some(workspace) = self.git_workspace_of_project(project.id, cx) {
            workspace.update(cx, |ws, cx| ws.reload_worktrees(cx));
        } else {
            self.send_command(GitCommand::ListWorktrees {
                project: project.clone(),
                request_id: GitRequestId::new(),
            });
        }
        cx.notify();
    }

    /// 把一个工作树目录加进项目列表（工作树视图点击即切换/导入）。
    pub(crate) fn git_open_worktree_path(&mut self, path: &str, cx: &mut Context<AppShell>) {
        let target = std::path::PathBuf::from(path);
        if let Some(project) = self
            .git_registry(cx)
            .projects
            .iter()
            .find(|item| item.path == target)
            .cloned()
        {
            self.git_refresh(&project, cx);
            return;
        }
        self.add_git_project(super::GitProjectDraft::new(target), cx);
    }

    /// 打开（或聚焦）一个项目的 Git 标签页。
    pub(crate) fn open_git_tab(
        &mut self,
        project: GitProject,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        self.update_git_registry(cx, |git| git.selected_project = Some(project.id));
        if let Some(tab_id) = self.git_tab_for_project(project.id, cx) {
            self.window_requests.push_activate_tab(tab_id);
            return;
        }
        let tab_id = TabId::new();
        let session = git_placeholder_session(&project);
        let session_id = session.id;
        let title = project.display_name();
        let state = GitWorkspaceState::new(project.clone(), window, cx);
        let command_tx = self.services.command_tx.clone();
        let workspace = cx.new(|_| GitWorkspace::new(state, tab_id, command_tx));
        let subscription = cx.subscribe(&workspace, |shell, _, event, cx| {
            GitActions::new(&mut shell.state).handle_git_workspace_event(event, cx);
        });
        self.tasks
            .subscriptions
            .git_workspaces
            .insert(tab_id, subscription);
        self.workspace.tabs.push(TerminalTab::git_tab(
            tab_id,
            session,
            title,
            workspace.clone(),
        ));
        self.workspace.create_group(tab_id, session_id);
        self.window_requests.push_activate_tab(tab_id);
        self.git_refresh(&project, cx);
        workspace.update(cx, |ws, cx| ws.load_log_notified(0, cx));
        cx.notify();
    }

    /// 消费 Git 工作区实体上报的跨域意图事件。
    pub(crate) fn handle_git_workspace_event(
        &mut self,
        event: &GitWorkspaceEvent,
        cx: &mut Context<AppShell>,
    ) {
        match event {
            GitWorkspaceEvent::TransientStatus(message) => {
                self.set_transient_status(message.clone());
            }
            GitWorkspaceEvent::Error(message) => {
                self.set_error(message.clone());
                cx.notify();
            }
            GitWorkspaceEvent::OpenPushModal { project, target } => {
                if crate::app::app_settings(cx).git.push_dialog {
                    crate::git::actions::modal_bridges::GitModalActions::from_actions(self)
                        .open_git_push_modal(project, target.clone(), cx);
                } else {
                    let options = transom_core::GitPushOptions {
                        branch: target.clone(),
                        ..Default::default()
                    };
                    self.send_command(GitCommand::Push {
                        project: project.clone(),
                        options,
                    });
                    cx.notify();
                }
            }
            GitWorkspaceEvent::OpenNameModal { .. }
            | GitWorkspaceEvent::OpenWorktreeModal { .. } => {
                self.window_requests.push_git(event.clone());
                cx.notify();
            }
            GitWorkspaceEvent::OpenRebaseModal { project, upstream } => {
                crate::git::actions::modal_bridges::GitModalActions::from_actions(self)
                    .open_git_rebase_modal(project, upstream, cx);
            }
            GitWorkspaceEvent::OpenConflictModal(project) => {
                crate::git::actions::modal_bridges::GitModalActions::from_actions(self)
                    .open_git_conflict_modal(project, cx);
            }
            GitWorkspaceEvent::OpenWorktreePath(path) => {
                self.git_open_worktree_path(path, cx);
            }
            GitWorkspaceEvent::ProjectInitStarted {
                request_id,
                project,
            } => {
                self.update_git_registry(cx, |git| {
                    git.pending_project_init = Some((*request_id, project.id));
                });
                cx.notify();
            }
            GitWorkspaceEvent::OpenSettings => {
                self.window_requests
                    .push_git(GitWorkspaceEvent::OpenSettings);
                cx.notify();
            }
            GitWorkspaceEvent::PersistSettings(settings) => {
                self.persist_settings_reported(settings);
                cx.notify();
            }
        }
    }

    /// 关闭标签页时取消该项目所有在途请求。
    pub(crate) fn cancel_git_tab(&mut self, tab_id: TabId, cx: &mut Context<AppShell>) {
        let Some(workspace) = self.git_workspace(tab_id) else {
            return;
        };
        workspace.read(cx).cancel_pending();
        cx.notify();
    }

    /// 按项目 id 找已打开的标签页。
    pub(crate) fn git_tab_for_project(&self, project_id: GitProjectId, cx: &App) -> Option<TabId> {
        self.git_workspace_of_project(project_id, cx)
            .map(|workspace| workspace.read(cx).tab_id)
    }

    /// 按草稿添加项目到列表（探测或初始化通过后调用）。
    pub(crate) fn add_git_project(&mut self, draft: GitProjectDraft, cx: &mut Context<AppShell>) {
        if self
            .git_registry(cx)
            .projects
            .iter()
            .any(|item| item.path == draft.path)
        {
            self.set_error(rust_i18n::t!("git.project.duplicate").to_string());
            return;
        }
        let project = draft.into_project();
        self.update_git_registry(cx, |git| {
            git.badges.insert(project.id, GitBadge::default());
            git.projects.push(project.clone());
        });
        self.persist_git_projects(cx);
        self.git_refresh(&project, cx);
    }

    /// 从列表移除项目（只删登记，不动磁盘）。
    pub(crate) fn remove_git_project(&mut self, id: GitProjectId, cx: &mut Context<AppShell>) {
        self.update_git_registry(cx, |git| {
            git.projects.retain(|project| project.id != id);
            git.badges.remove(&id);
            if git.selected_project == Some(id) {
                git.selected_project = None;
            }
        });
        if let Some(tab_id) = self.git_tab_for_project(id, cx) {
            self.cancel_git_tab(tab_id, cx);
        }
        self.persist_git_projects(cx);
        cx.notify();
    }

    /// 打开偏好设置弹窗并直接定位到 Git 版本控制设置页。
    pub(crate) fn open_git_settings(&mut self, _window: &mut Window, _cx: &mut Context<AppShell>) {
        self.window_requests
            .push_open_settings(Some(crate::settings::GIT_PAGE_INDEX));
    }
}

/// Git 标签页需要的占位会话。
///
/// 设计原理（Why）：工作区的标签、分组与关闭逻辑都以 `Session` 为轴，Git 面板
/// 不是终端但必须复用这套容器；用 `Connection::Local` 且工作目录指向仓库根，
/// 语义上就是「这个仓库在本机」，不引入第三种连接类型去牵动转发/SFTP 的分支。
fn git_placeholder_session(project: &GitProject) -> Session {
    Session::new(
        project.display_name(),
        Connection::Local(LocalConnection {
            shell: None,
            shell_args: Vec::new(),
            run_as_admin: false,
            working_dir: Some(project.path.clone()),
            command: None,
            ..Default::default()
        }),
    )
}
