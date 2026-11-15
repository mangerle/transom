//! Git 分支、标签、工作树与远端写命令处理。

//! Git 写操作命令处理。

use super::super::state::{ControllerCtx, ControllerState};
use super::runner::{send_event, spawn_write, to_app_error};
use transom_core::{GitAction, GitAppEvent, GitCommand};

/// 处理分支、标签与工作树（Worktree）相关 Git 写操作命令。
pub(super) fn handle_refs_and_worktrees(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: GitCommand,
) {
    match command {
        GitCommand::Checkout { project, target } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Checkout,
                move |repo| async move {
                    match target {
                        transom_core::GitCheckout::Branch(branch) => repo.checkout(&branch).await,
                        transom_core::GitCheckout::Detached(oid) => {
                            repo.checkout_detached(&oid).await
                        }
                    }
                },
            );
        }
        GitCommand::CreateBranch {
            project,
            name,
            start,
            switch_to,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::CreateBranch,
                move |repo| async move { repo.create_branch(&name, start.as_deref(), switch_to).await },
            );
        }
        GitCommand::RenameBranch { project, old, new } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::RenameBranch,
                move |repo| async move { repo.rename_branch(&old, &new).await },
            );
        }
        GitCommand::DeleteBranch {
            project,
            name,
            force,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::DeleteBranch,
                move |repo| async move { repo.delete_branch(&name, force).await },
            );
        }
        GitCommand::CreateTag {
            project,
            name,
            target,
            message,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::CreateTag,
                move |repo| async move { repo.create_tag(&name, &target, message.as_deref()).await },
            );
        }
        GitCommand::DeleteTag { project, name } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::DeleteTag,
                move |repo| async move { repo.delete_tag(&name).await },
            );
        }
        GitCommand::AddWorktree {
            project,
            path,
            branch,
            start,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::AddWorktree,
                move |repo| async move { repo.add_worktree(&path, &branch, start.as_deref()).await },
            );
        }
        GitCommand::RemoveWorktree {
            project,
            path,
            force,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::RemoveWorktree,
                move |repo| async move { repo.remove_worktree(&path, force).await },
            );
        }
        GitCommand::LockWorktree {
            project,
            path,
            reason,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::LockWorktree,
                move |repo| async move { repo.lock_worktree(&path, reason.as_deref()).await },
            );
        }
        GitCommand::UnlockWorktree { project, path } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::UnlockWorktree,
                move |repo| async move { repo.unlock_worktree(&path).await },
            );
        }
        _ => {}
    }
}

/// 处理远端同步与仓库初始化命令。
pub(super) fn handle_remote_and_init(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: GitCommand,
) {
    match command {
        GitCommand::Fetch { project } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Fetch,
                move |repo| async move { repo.fetch().await },
            );
        }
        GitCommand::Push { project, options } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Push,
                move |repo| async move { repo.push_advanced(&options).await },
            );
        }
        GitCommand::PushUpTo {
            project,
            oid,
            branch,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::PushUpTo,
                move |repo| async move { repo.push_up_to(&oid, &branch).await },
            );
        }
        GitCommand::Pull { project, rebase } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Pull,
                move |repo| async move { repo.pull(rebase).await },
            );
        }
        GitCommand::Init { request_id, path } => {
            let config = ctx.git_config_snapshot();
            let event_tx = ctx.event_tx.clone();
            state.git_set.spawn(async move {
                let error = match transom_git::init(&path, &config).await {
                    Ok(_) => None,
                    Err(error) => {
                        log::error!(
                            "Git 仓库初始化失败，路径: {}，原因: {error}",
                            path.display()
                        );
                        Some(to_app_error(error))
                    }
                };
                send_event(
                    &event_tx,
                    GitAppEvent::Inited {
                        request_id,
                        path,
                        error,
                    },
                );
            });
        }
        _ => {}
    }
}
