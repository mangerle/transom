//! Git 事件落地：把控制器回传的事件写进对应标签页与侧栏徽标。
//!
//! 设计原理（Why）：事件处理是「唯一写界面的入口」，单独成模块便于逐条核对
//! 迟到请求的丢弃口径（按 `request_id` 比对）与写动作后的自动刷新，
//! 不让这些规则散落在各个点击回调里。
//!
//! 状态写入一律走 `Entity<GitWorkspace>::update`：闭包内只改该标签页的状态并把
//! 外壳后续需要的数据带出来，闭包结束后再调用外壳方法，避免同一实体被二次租借。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::GitAppEvent;

/// Git 事件落地。
pub(crate) fn dispatch_git_event(
    shell: &mut AppShell,
    event: GitAppEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        GitAppEvent::StatusLoaded { project_id, status } => {
            action_done::apply_git_status(shell, project_id, *status, cx);
        }
        GitAppEvent::RefsLoaded { .. }
        | GitAppEvent::LogLoaded { .. }
        | GitAppEvent::RebaseTodoLoaded { .. }
        | GitAppEvent::CommitUrlLoaded { .. }
        | GitAppEvent::CommitFilesLoaded { .. }
        | GitAppEvent::OutgoingCommitsLoaded { .. }
        | GitAppEvent::DiffLoaded { .. }
        | GitAppEvent::ReadFailed { .. } => {
            read_events::handle_read_event(shell, event, cx);
        }
        GitAppEvent::ActionDone {
            project_id,
            action,
            error,
        } => {
            action_done::handle_git_action_done(shell, project_id, action, error, cx);
        }
        GitAppEvent::WorktreesLoaded { .. }
        | GitAppEvent::StashesLoaded { .. }
        | GitAppEvent::StashFilesLoaded { .. }
        | GitAppEvent::ShelvesLoaded { .. }
        | GitAppEvent::Shelved { .. }
        | GitAppEvent::ShelfApplied { .. }
        | GitAppEvent::ShelfDropped { .. }
        | GitAppEvent::ShelfFailed { .. } => {
            shelf_events::handle_shelf_worktree_event(shell, event, cx);
        }
        GitAppEvent::Probed { .. } => {
            crate::git::actions::modal_bridges::apply_git_probe_event(shell, event, cx);
        }
        GitAppEvent::Inited {
            request_id,
            path,
            error,
        } => {
            let ours = shell
                .state
                .git
                .pending_project_init
                .is_some_and(|(pending_id, _)| pending_id == request_id);
            if ours {
                super::init_event::apply_git_project_init_event(
                    shell, request_id, &path, error, cx,
                );
            } else {
                crate::git::actions::modal_bridges::apply_git_probe_event(
                    shell,
                    GitAppEvent::Inited {
                        request_id,
                        path,
                        error,
                    },
                    cx,
                );
            }
        }
        GitAppEvent::CloneProgress { .. } | GitAppEvent::CloneDone { .. } => {
            crate::git::clone_modal::apply_git_clone_event(shell, event, cx);
        }
        GitAppEvent::ConflictSidesLoaded {
            project_id,
            path,
            sides,
            ..
        } => {
            crate::git::actions::modal_bridges::apply_git_conflict_sides(
                shell, project_id, &path, *sides, cx,
            );
        }
    }
}

mod action_done;
mod read_events;
mod shelf_events;
