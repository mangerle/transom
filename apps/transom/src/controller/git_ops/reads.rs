//! Git 只读查询命令处理。

use std::sync::Arc;

use super::super::state::{ControllerCtx, ControllerState};
use super::runner::{send_event, spawn_read, to_app_error, to_side};
use transom_core::{GitAppEvent, GitCommand, GitRequestId};

/// 处理 Git 只读查询命令；若是只读命令则返回 `true`，否则返回 `false`。
pub(super) fn handle_read_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: &GitCommand,
) -> bool {
    match command {
        GitCommand::Refresh { project } => {
            let project_id = project.id;
            let project = project.clone();
            spawn_read(
                state,
                ctx,
                project,
                GitRequestId::new(),
                // 引擎支持读忽略清单，这里恒传 `false`：面板口径与 `git status` 一致。
                move |repo| async move {
                    Ok(GitAppEvent::StatusLoaded {
                        project_id,
                        status: Box::new(repo.status(false).await?),
                    })
                },
            );
            true
        }
        GitCommand::LoadLog {
            project,
            request_id,
            query,
        } => {
            let project_id = project.id;
            let project_path = project.path.clone();
            let project = project.clone();
            let request_id = *request_id;
            let query = query.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let page = repo.log(&query).await?;
                // 里程碑级诊断：界面若出现「提交表空白」，这条日志能立刻区分
                // 「没取到数据」与「取到了但渲染不下」。一次历史加载一行，不是高频输出。
                log::info!(
                    "提交历史已读取，项目路径: {}，条数: {}，是否还有更多: {}，跳过: {}",
                    project_path.display(),
                    page.entries.len(),
                    page.has_more,
                    query.skip
                );
                Ok(GitAppEvent::LogLoaded {
                    project_id,
                    request_id,
                    page: Box::new(page),
                })
            });
            true
        }
        GitCommand::LoadCommitFiles {
            project,
            request_id,
            oid,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let oid = oid.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let files = repo.commit_files(&oid).await?;
                Ok(GitAppEvent::CommitFilesLoaded {
                    project_id,
                    request_id,
                    oid,
                    files,
                })
            });
            true
        }
        GitCommand::LoadOutgoingCommits {
            project,
            request_id,
            branch,
            remote,
            remote_branch,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let branch = branch.clone();
            let remote = remote.clone();
            let remote_branch = remote_branch.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let commits = repo
                    .outgoing_commits(&branch, remote.as_deref(), remote_branch.as_deref())
                    .await?;
                let effective_remote_branch = remote_branch.unwrap_or_else(|| branch.clone());
                Ok(GitAppEvent::OutgoingCommitsLoaded {
                    project_id,
                    request_id,
                    branch,
                    remote_branch: effective_remote_branch,
                    commits,
                })
            });
            true
        }
        GitCommand::LoadRefs { project } => {
            let project_id = project.id;
            let project = project.clone();
            spawn_read(
                state,
                ctx,
                project,
                GitRequestId::new(),
                move |repo| async move {
                    let branches = repo.branches().await?;
                    let tags = repo.tags().await?;
                    let paths = repo.tracked_files().await?;
                    Ok(GitAppEvent::RefsLoaded {
                        project_id,
                        branches,
                        tags,
                        paths,
                    })
                },
            );
            true
        }
        GitCommand::LoadDiff {
            project,
            request_id,
            path,
            old,
            new,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let path = path.clone();
            let old_side = to_side(old.clone());
            let new_side = to_side(new.clone());
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let diff = repo.diff(&old_side, &new_side, &path).await?;
                Ok(GitAppEvent::DiffLoaded {
                    project_id,
                    request_id,
                    diff: Box::new(diff),
                })
            });
            true
        }
        GitCommand::Probe { request_id, path } => {
            let request_id = *request_id;
            let path = path.clone();
            let config = ctx.git_config_snapshot();
            let service = Arc::clone(&ctx.git);
            let event_tx = ctx.event_tx.clone();
            let flag = service.register(request_id);
            state.git_set.spawn(async move {
                let (info, error) = match transom_git::probe(&path, &config).await {
                    Ok(info) => (Some(Box::new(info)), None),
                    Err(transom_git::GitError::NotRepo { .. }) => (None, None),
                    Err(error) => (None, Some(to_app_error(error))),
                };
                service.finish(request_id);
                if !transom_git::GitService::is_cancelled(&flag) {
                    send_event(
                        &event_tx,
                        GitAppEvent::Probed {
                            request_id,
                            path,
                            info,
                            error,
                        },
                    );
                }
            });
            true
        }
        GitCommand::ListWorktrees {
            project,
            request_id,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let entries = repo.worktrees().await?;
                Ok(GitAppEvent::WorktreesLoaded {
                    project_id,
                    request_id,
                    entries,
                })
            });
            true
        }
        GitCommand::RebaseTodo {
            project,
            request_id,
            base,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let base_str = base.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let steps = repo.rebase_todo(&base_str).await?;
                Ok(GitAppEvent::RebaseTodoLoaded {
                    project_id,
                    request_id,
                    base: base_str,
                    steps,
                })
            });
            true
        }
        GitCommand::ListStashes {
            project,
            request_id,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let entries = repo.stashes().await?;
                Ok(GitAppEvent::StashesLoaded {
                    project_id,
                    request_id,
                    entries,
                })
            });
            true
        }
        GitCommand::ListStashFiles {
            project,
            request_id,
            selector,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let sel = selector.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let files = repo.stash_files(&sel).await?;
                Ok(GitAppEvent::StashFilesLoaded {
                    project_id,
                    request_id,
                    selector: sel,
                    files,
                })
            });
            true
        }
        GitCommand::ResolveCommitUrl {
            project,
            request_id,
            oid,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let oid = oid.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let url = repo.commit_url(&oid).await?;
                Ok(GitAppEvent::CommitUrlLoaded {
                    project_id,
                    request_id,
                    oid,
                    url,
                })
            });
            true
        }
        GitCommand::LoadConflictSides {
            project,
            request_id,
            path,
        } => {
            let project_id = project.id;
            let project = project.clone();
            let request_id = *request_id;
            let path = path.clone();
            spawn_read(state, ctx, project, request_id, move |repo| async move {
                let sides = repo.conflict_sides(&path).await?;
                Ok(GitAppEvent::ConflictSidesLoaded {
                    project_id,
                    request_id,
                    path,
                    sides: Box::new(sides),
                })
            });
            true
        }
        GitCommand::Cancel { request_id } => {
            if !ctx.git.cancel(*request_id) {
                log::debug!("取消的 Git 请求已结束或不存在，请求: {request_id}");
            }
            true
        }
        _ => false,
    }
}
