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
use crate::app::{AppShell, TerminalTab};
use crate::palette::localized_error;
use crate::sidebar::SidebarSection;
use gpui_kit::*;
use transom_core::{
    Connection, GitCommand, GitProject, GitProjectId, GitRequestId, LocalConnection, Session,
    TabId,
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
mod ref_actions;
mod shelf_actions;
pub(crate) use shelf_actions::CreateWorktreeParams;
mod sidebar_actions;
mod workspace_refs;
pub(crate) use sidebar_actions::{
    add_git_folder, apply_git_drop, delete_git_folder, git_move_project_to_root, rename_git_folder,
    select_git_project, toggle_git_folder_collapsed,
};

// 统一收口对外导出的动作与事件分发自由函数，供 Git 各视图与主壳层按 `crate::git::actions::*` 调用。
pub(crate) use super::types::{GitDiffRequest, first_worktree_diff};
pub(crate) use ref_actions::GitHistoryAction;

/// 项目与分组一次落盘。
///
/// 设计原理（Why）：两者必须同一次写完——分开保存会出现「分组已删、
/// 项目的 `folder_id` 还指着已消失分组」这种撕裂状态（同 `SessionStore` 的理由）。
pub(crate) fn persist_git_projects(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let registry = transom_core::GitRegistry {
        projects: shell.state.git.projects.clone(),
        folders: shell.state.git.folders.clone(),
    };
    if let Err(error) = shell.state.services.git_projects.save(&registry) {
        shell.set_error(localized_error(&error));
    }
    cx.notify();
}

/// 活动栏点击：切换左侧槽位的 Git 项目列表。
pub(crate) fn toggle_git_sidebar(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let opening = shell.state.sidebar.toggle_section(SidebarSection::Git);
    if opening {
        // 展开即刷一遍所有项目：列表里的分支与脏数必须是当下的。
        refresh_all_git_projects(shell, cx);
    }
    cx.notify();
}

/// 刷新全部已登记项目的状态徽标。
pub(crate) fn refresh_all_git_projects(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    for index in 0..shell.state.git.projects.len() {
        let project = shell.state.git.projects[index].clone();
        git_refresh(shell, &project, cx);
    }
}

/// 刷新所有处于管理状态的 Git 项目状态（用于窗口重新聚焦时自动刷新）。
pub(crate) fn git_refresh_active_projects(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    refresh_all_git_projects(shell, cx);
}

/// 采集单个项目的状态快照（侧栏徽标与标签页共用同一条命令）。
pub(crate) fn git_refresh(shell: &mut AppShell, project: &GitProject, cx: &mut Context<AppShell>) {
    let badge = shell.state.git.badges.entry(project.id).or_default();
    badge.loading = true;
    badge.error = None;
    shell.send_command(GitCommand::Refresh {
        project: project.clone(),
    });
    shell.send_command(GitCommand::LoadRefs {
        project: project.clone(),
    });
    if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
        workspace.update(cx, |ws, cx| ws.reload_worktrees(cx));
    } else {
        shell.send_command(GitCommand::ListWorktrees {
            project: project.clone(),
            request_id: GitRequestId::new(),
        });
    }
    cx.notify();
}

/// 把一个工作树目录加进项目列表（工作树视图点击即切换/导入）。
fn git_open_worktree_path(shell: &mut AppShell, path: &str, cx: &mut Context<AppShell>) {
    let target = std::path::PathBuf::from(path);
    if let Some(project) = shell
        .state
        .git
        .projects
        .iter()
        .find(|item| item.path == target)
        .cloned()
    {
        git_refresh(shell, &project, cx);
        return;
    }
    add_git_project(shell, super::GitProjectDraft::new(target), cx);
}

/// 打开（或聚焦）一个项目的 Git 标签页。
pub(crate) fn open_git_tab(
    shell: &mut AppShell,
    project: GitProject,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    shell.state.git.selected_project = Some(project.id);
    if let Some(tab_id) = git_tab_for_project(shell, project.id, cx) {
        shell.activate_tab(tab_id, window, cx);
        return;
    }
    let tab_id = TabId::new();
    let session = git_placeholder_session(&project);
    let session_id = session.id;
    let title = project.display_name();
    let state = GitWorkspaceState::new(project.clone(), window, cx);
    let command_tx = shell.state.services.command_tx.clone();
    let workspace = cx.new(|_| GitWorkspace::new(state, tab_id, command_tx));
    let subscription = cx.subscribe(&workspace, |shell, _, event, cx| {
        handle_git_workspace_event(shell, event, cx);
    });
    shell
        .state
        .tasks
        .subscriptions
        .git_workspaces
        .insert(tab_id, subscription);
    shell
        .state
        .workspace
        .tabs
        .push(TerminalTab::git_tab(tab_id, session, title, workspace.clone()));
    shell.state.workspace.create_group(tab_id, session_id);
    shell.activate_tab(tab_id, window, cx);
    git_refresh(shell, &project, cx);
    workspace.update(cx, |ws, cx| ws.load_log_notified(0, cx));
    cx.notify();
}

/// 消费 Git 工作区实体上报的跨域意图事件。
fn handle_git_workspace_event(
    shell: &mut AppShell,
    event: &GitWorkspaceEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitWorkspaceEvent::TransientStatus(message) => {
            shell.set_transient_status(message.clone(), cx);
        }
        GitWorkspaceEvent::Error(message) => {
            shell.set_error(message.clone());
            cx.notify();
        }
        GitWorkspaceEvent::OpenPushModal { project, target } => {
            if crate::app::app_settings(cx).git.push_dialog {
                modal_bridges::open_git_push_modal(shell, project, target.clone(), cx);
            } else {
                let options = transom_core::GitPushOptions {
                    branch: target.clone(),
                    ..Default::default()
                };
                shell.send_command(GitCommand::Push {
                    project: project.clone(),
                    options,
                });
                cx.notify();
            }
        }
        GitWorkspaceEvent::OpenNameModal { .. } | GitWorkspaceEvent::OpenWorktreeModal { .. } => {
            shell.state.window_requests.push_git(event.clone());
            cx.notify();
        }
        GitWorkspaceEvent::OpenRebaseModal { project, upstream } => {
            modal_bridges::open_git_rebase_modal(shell, project, upstream, cx);
        }
        GitWorkspaceEvent::OpenConflictModal(project) => {
            modal_bridges::open_git_conflict_modal(shell, project, cx);
        }
        GitWorkspaceEvent::OpenWorktreePath(path) => {
            git_open_worktree_path(shell, path, cx);
        }
        GitWorkspaceEvent::CreatePatch { project, path } => {
            crate::git::patch::git_create_patch(shell, project, path, cx);
        }
        GitWorkspaceEvent::BrowseCommit { project, oid } => {
            crate::git::web_link::git_browse_commit(shell, project, oid, cx);
        }
        GitWorkspaceEvent::ProjectInitStarted {
            request_id,
            project,
        } => {
            shell.state.git.pending_project_init = Some((*request_id, project.id));
            cx.notify();
        }
        GitWorkspaceEvent::OpenSettings => {
            shell
                .state
                .window_requests
                .push_git(GitWorkspaceEvent::OpenSettings);
            cx.notify();
        }
        GitWorkspaceEvent::PersistSettings(settings) => {
            shell.state.persist_settings_reported(settings);
            cx.notify();
        }
    }
}

/// 关闭标签页时取消该项目所有在途请求。
pub(crate) fn cancel_git_tab(shell: &mut AppShell, tab_id: TabId, cx: &mut Context<AppShell>) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.read(cx).cancel_pending();
    cx.notify();
}

/// 按项目 id 找已打开的标签页。
pub(crate) fn git_tab_for_project(
    shell: &AppShell,
    project_id: GitProjectId,
    cx: &App,
) -> Option<TabId> {
    shell
        .state
        .git_workspace_of_project(project_id, cx)
        .map(|workspace| workspace.read(cx).tab_id)
}

/// 按草稿添加项目到列表（探测或初始化通过后调用）。
pub(crate) fn add_git_project(
    shell: &mut AppShell,
    draft: GitProjectDraft,
    cx: &mut Context<AppShell>,
) {
    if shell
        .state
        .git
        .projects
        .iter()
        .any(|item| item.path == draft.path)
    {
        shell.set_error(rust_i18n::t!("git.project.duplicate").to_string());
        return;
    }
    let project = draft.into_project();
    shell
        .state
        .git
        .badges
        .insert(project.id, GitBadge::default());
    shell.state.git.projects.push(project.clone());
    persist_git_projects(shell, cx);
    git_refresh(shell, &project, cx);
}

/// 从列表移除项目（只删登记，不动磁盘）。
pub(crate) fn remove_git_project(
    shell: &mut AppShell,
    id: GitProjectId,
    cx: &mut Context<AppShell>,
) {
    shell.state.git.projects.retain(|project| project.id != id);
    shell.state.git.badges.remove(&id);
    if shell.state.git.selected_project == Some(id) {
        shell.state.git.selected_project = None;
    }
    if let Some(tab_id) = git_tab_for_project(shell, id, cx) {
        cancel_git_tab(shell, tab_id, cx);
    }
    persist_git_projects(shell, cx);
    cx.notify();
}

/// 打开偏好设置弹窗并直接定位到 Git 版本控制设置页。
pub(crate) fn open_git_settings(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    shell.open_settings_modal(Some(crate::settings::GIT_PAGE_INDEX), window, cx);
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
