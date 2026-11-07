//! Git 异步任务派发与事件发送辅助。

use std::future::Future;
use std::sync::Arc;

use super::super::state::{ControllerCtx, ControllerState};
use transom_core::{AppError, AppEvent, GitAction, GitAppEvent, GitProject, GitRequestId, GitSide};
use transom_git::{GitError, GitService, Repo, Side};

/// 派发一次读请求。
///
/// `project_id` 与 `request_id` 由调用点自行捕获：它们都是 `Copy`，在闭包里
/// 直接引用比再造一个上下文包装类型更直白。
pub(super) fn spawn_read<F, Fut>(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    project: GitProject,
    request_id: GitRequestId,
    work: F,
) where
    F: FnOnce(Repo) -> Fut + Send + 'static,
    Fut: Future<Output = Result<GitAppEvent, GitError>> + Send + 'static,
{
    let service = Arc::clone(&ctx.git);
    let event_tx = ctx.event_tx.clone();
    let project_id = project.id;
    let flag = service.register(request_id);
    state.git_set.spawn(async move {
        let event = match work(Repo::at(project.path.clone())).await {
            Ok(event) => Some(event),
            Err(error) => {
                if error.is_not_repo() {
                    log::debug!("Git 目录尚未初始化为仓库，项目: {}", project.path.display());
                } else {
                    log::warn!(
                        "Git 读取请求失败，项目: {}，原因: {error}",
                        project.path.display()
                    );
                }
                Some(GitAppEvent::ReadFailed {
                    project_id,
                    request_id,
                    error: to_app_error(error),
                })
            }
        };
        service.finish(request_id);
        if GitService::is_cancelled(&flag) {
            log::debug!("Git 读请求已取消，结果被丢弃，项目: {project_id}");
            return;
        }
        if let Some(event) = event {
            send_event(&event_tx, event);
        }
    });
}

/// 派发一次写动作。
pub(super) fn spawn_write<F, Fut>(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    project: GitProject,
    action: GitAction,
    work: F,
) where
    F: FnOnce(Repo) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), GitError>> + Send + 'static,
{
    let event_tx = ctx.event_tx.clone();
    let project_id = project.id;
    state.git_set.spawn(async move {
        let error = match work(Repo::at(project.path.clone())).await {
            Ok(()) => None,
            Err(error) => {
                log::error!(
                    "Git 写动作失败，动作: {:?}，项目: {}，原因: {error}",
                    action,
                    project.path.display()
                );
                Some(to_app_error(error))
            }
        };
        send_event(
            &event_tx,
            GitAppEvent::ActionDone {
                project_id,
                action,
                error,
            },
        );
    });
}

/// core 侧的差异来源枚举转引擎侧别名（`Side` 是 `GitSide` 的再导出，
/// 这里显式匹配以便 core 契约扩展时编译期暴露漏项）。
pub(super) fn to_side(side: GitSide) -> Side {
    match side {
        GitSide::Worktree => Side::Worktree,
        GitSide::Index => Side::Index,
        GitSide::Commit(oid) => Side::Commit(oid),
    }
}

pub(super) fn to_app_error(error: GitError) -> AppError {
    AppError::from(error)
}

/// 事件投递：控制平面永不因界面背压而阻塞，队列满时丢弃并记录。
pub(super) fn send_event(event_tx: &flume::Sender<AppEvent>, event: GitAppEvent) {
    if let Err(error) = event_tx.try_send(event.into()) {
        log::warn!("Git 事件投递失败（界面处理过慢），原因: {error}");
    }
}
