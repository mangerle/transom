//! Git 写操作命令处理。

use super::super::state::{ControllerCtx, ControllerState};
use super::runner::spawn_write;
use transom_core::{GitAction, GitCommand};

/// 处理 Git 写操作命令。
pub(super) fn handle_write_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: GitCommand,
) {
    match command {
        GitCommand::Stage { .. }
        | GitCommand::StageAll { .. }
        | GitCommand::Unstage { .. }
        | GitCommand::Discard { .. }
        | GitCommand::ApplyHunkPatch { .. }
        | GitCommand::DeleteUntracked { .. }
        | GitCommand::Commit { .. }
        | GitCommand::CreatePatch { .. }
        | GitCommand::ResolveConflictOurs { .. }
        | GitCommand::ResolveConflictTheirs { .. }
        | GitCommand::ResolveConflictMark { .. } => {
            handle_working_tree_and_commit(state, ctx, command);
        }
        GitCommand::Checkout { .. }
        | GitCommand::CreateBranch { .. }
        | GitCommand::RenameBranch { .. }
        | GitCommand::DeleteBranch { .. }
        | GitCommand::CreateTag { .. }
        | GitCommand::DeleteTag { .. }
        | GitCommand::AddWorktree { .. }
        | GitCommand::RemoveWorktree { .. }
        | GitCommand::LockWorktree { .. }
        | GitCommand::UnlockWorktree { .. } => {
            super::write_refs::handle_refs_and_worktrees(state, ctx, command);
        }
        GitCommand::Fetch { .. }
        | GitCommand::Push { .. }
        | GitCommand::PushUpTo { .. }
        | GitCommand::Pull { .. }
        | GitCommand::Init { .. } => {
            super::write_refs::handle_remote_and_init(state, ctx, command);
        }
        GitCommand::ResetTo { .. }
        | GitCommand::Revert { .. }
        | GitCommand::CherryPick { .. }
        | GitCommand::Merge { .. }
        | GitCommand::RebaseOnto { .. }
        | GitCommand::RebaseAbort { .. }
        | GitCommand::RebaseRun { .. }
        | GitCommand::FoldTipIntoParent { .. }
        | GitCommand::StashSave { .. }
        | GitCommand::StashApply { .. }
        | GitCommand::StashPop { .. }
        | GitCommand::StashDrop { .. } => {
            handle_history_and_stash(state, ctx, command);
        }
        _ => {}
    }
}

/// 处理工作区暂存、提交、补丁与冲突解决命令。
fn handle_working_tree_and_commit(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: GitCommand,
) {
    match command {
        GitCommand::Stage { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Stage,
                move |repo| async move { repo.stage(&paths).await },
            );
        }
        GitCommand::StageAll { project } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::StageAll,
                move |repo| async move { repo.stage_all().await },
            );
        }
        GitCommand::Unstage { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Unstage,
                move |repo| async move { repo.unstage(&paths).await },
            );
        }
        GitCommand::Discard { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Discard,
                move |repo| async move { repo.discard(&paths).await },
            );
        }
        GitCommand::ApplyHunkPatch {
            project,
            action,
            patch,
            cached,
            reverse,
        } => {
            spawn_write(state, ctx, project, action, move |repo| async move {
                repo.apply_hunk_patch(&patch, cached, reverse).await
            });
        }
        GitCommand::DeleteUntracked { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::DeleteUntracked,
                move |repo| async move { repo.discard_untracked(&paths).await },
            );
        }
        GitCommand::Commit {
            project,
            message,
            amend,
            fixup_target,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Commit,
                move |repo| async move {
                    if let Some(target) = fixup_target {
                        repo.commit_fixup(&target).await
                    } else {
                        repo.commit(&message, amend).await
                    }
                },
            );
        }
        GitCommand::CreatePatch { project, oid, path } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::CreatePatch,
                move |repo| async move {
                    let content = repo.commit_patch(&oid).await?;
                    tokio::fs::write(&path, content).await.map_err(|source| {
                        transom_git::GitError::Spawn {
                            command: format!("写入补丁 {}", path.display()),
                            source,
                        }
                    })
                },
            );
        }
        GitCommand::ResolveConflictOurs { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::ResolveConflictOurs,
                move |repo| async move { repo.resolve_ours(&paths).await },
            );
        }
        GitCommand::ResolveConflictTheirs { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::ResolveConflictTheirs,
                move |repo| async move { repo.resolve_theirs(&paths).await },
            );
        }
        GitCommand::ResolveConflictMark { project, paths } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::ResolveConflictMark,
                move |repo| async move { repo.mark_resolved(&paths).await },
            );
        }
        _ => {}
    }
}

/// 处理分支、标签与工作树（Worktree）变更命令。
fn handle_history_and_stash(state: &mut ControllerState, ctx: &ControllerCtx, command: GitCommand) {
    match command {
        GitCommand::ResetTo {
            project,
            target,
            kind,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Reset,
                move |repo| async move { repo.reset(&target, kind).await },
            );
        }
        GitCommand::Revert { project, oid } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Revert,
                move |repo| async move { repo.revert(&oid).await },
            );
        }
        GitCommand::CherryPick { project, oid } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::CherryPick,
                move |repo| async move { repo.cherry_pick(&oid).await },
            );
        }
        GitCommand::Merge { project, reference } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Merge,
                move |repo| async move { repo.merge(&reference).await },
            );
        }
        GitCommand::RebaseOnto { project, upstream } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::Rebase,
                move |repo| async move { repo.rebase_onto(&upstream).await },
            );
        }
        GitCommand::RebaseAbort { project } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::RebaseAbort,
                move |repo| async move { repo.rebase_abort().await },
            );
        }
        GitCommand::RebaseRun {
            project,
            base,
            steps,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::RebaseRun,
                move |repo| async move { repo.rebase_with_todo(&base, &steps).await },
            );
        }
        GitCommand::FoldTipIntoParent { project, mode } => {
            let action = match mode {
                transom_core::GitFoldMode::Squash => GitAction::SquashParent,
                transom_core::GitFoldMode::Fixup => GitAction::FixupParent,
            };
            spawn_write(state, ctx, project, action, move |repo| async move {
                repo.fold_tip_into_parent(mode).await
            });
        }
        GitCommand::StashSave {
            project,
            message,
            include_untracked,
        } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::StashSave,
                move |repo| async move { repo.stash_save(&message, include_untracked).await },
            );
        }
        GitCommand::StashApply { project, selector } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::StashApply,
                move |repo| async move { repo.stash_apply(&selector).await },
            );
        }
        GitCommand::StashPop { project, selector } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::StashPop,
                move |repo| async move { repo.stash_pop(&selector).await },
            );
        }
        GitCommand::StashDrop { project, selector } => {
            spawn_write(
                state,
                ctx,
                project,
                GitAction::StashDrop,
                move |repo| async move { repo.stash_drop(&selector).await },
            );
        }
        _ => {}
    }
}
