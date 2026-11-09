//! 镜像远端搜索与下载任务进度/完成事件处理。
//!
//! 设计原理（Why）：
//! - 将镜像搜索结果回填至对应工作区的 `hub_dialog` 状态中；
//! - 下载分层流式事件 `PullTaskProgress` 实时更新 `ImagePullManager` 的分层字节与百分比；
//! - `PullTaskFinished` 触发时自动调度队列中下一个处于排队中的镜像下载任务，
//!   并在镜像拉取成功后自动触发本地镜像列表刷新。

use super::*;
use transom_core::{ContainerRequestId, ImageLayerProgress, ImageSearchResult};

impl AppShell {
    /// 处理远端镜像搜索返回事件。
    pub(crate) fn on_container_images_searched(
        &mut self,
        request_id: ContainerRequestId,
        results: Vec<ImageSearchResult>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
            return;
        };
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            let applied = workspace.update(cx, |workspace, _| {
                workspace.apply_images_searched(request_id, results)
            });
            if applied {
                cx.notify();
            }
        }
    }

    /// 处理 Registry 实时标签列表返回事件。
    pub(crate) fn on_container_image_tags_listed(
        &mut self,
        request_id: ContainerRequestId,
        tags: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
            return;
        };
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            let applied = workspace.update(cx, |workspace, _| {
                workspace.apply_image_tags_listed(request_id, tags)
            });
            if applied {
                cx.notify();
            }
        }
    }

    /// 处理分层下载进度推送。
    pub(crate) fn on_container_pull_task_progress(
        &mut self,
        task_id: String,
        status: String,
        layers: Vec<ImageLayerProgress>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_ids: Vec<TabId> = self.state.workspace.tabs.iter().map(|t| t.id).collect();
        let target = tab_ids.into_iter().find(|tab_id| {
            self.state
                .container_workspace(*tab_id)
                .is_some_and(|workspace| {
                    workspace
                        .read(cx)
                        .state
                        .pull_manager
                        .get(&task_id)
                        .is_some()
                })
        });
        if let Some(tab_id) = target
            && let Some(workspace) = self.state.container_workspace(tab_id)
        {
            workspace.update(cx, |workspace, cx| {
                workspace.apply_pull_progress(&task_id, status, layers);
                cx.notify();
            });
        }
    }

    /// 处理镜像下载任务结束（成功、失败或取消）。
    pub(crate) fn on_container_pull_task_finished(
        &mut self,
        task_id: String,
        error: Option<transom_core::AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut target_tab = None;
        let mut next_command = None;
        let mut task_reference = String::new();
        let is_cancelled = error
            .as_ref()
            .is_some_and(|e| e.code == transom_core::ErrorCode::Cancelled);
        let error_msg = error.as_ref().map(localized_error);

        let tab_ids: Vec<TabId> = self.state.workspace.tabs.iter().map(|t| t.id).collect();
        for tab_id in tab_ids {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                continue;
            };
            let found = workspace.update(cx, |workspace, _| {
                workspace.apply_pull_finished(&task_id, error_msg.clone())
            });
            if let Some((reference, next)) = found {
                target_tab = Some(tab_id);
                task_reference = reference;
                next_command = next;
                break;
            }
        }

        if let Some(tab_id) = target_tab {
            if is_cancelled {
                // 用户主动取消，不弹出失败提示
            } else if let Some(err) = error_msg {
                let msg = rust_i18n::t!(
                    "container.pull_manager.task_failed",
                    reference = task_reference,
                    error = err
                )
                .to_string();
                self.show_toast(msg, false, window, cx);
            } else {
                let msg = rust_i18n::t!(
                    "container.pull_manager.task_finished",
                    reference = task_reference
                )
                .to_string();
                self.show_toast(msg, true, window, cx);
                // 自动刷新镜像列表以呈现新载入的镜像
                let req_images = ContainerRequestId::new();
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.send_tracked(
                            req_images,
                            ContainerCommand::ListImages {
                                tab_id,
                                request_id: req_images,
                            },
                        );
                    });
                }
            }

            // 调度排队中的下一个下载任务
            if let Some((next_task_id, next_tab_id, reference, auth)) = next_command {
                self.state.send_command(ContainerCommand::PullImageTask {
                    tab_id: next_tab_id,
                    task_id: next_task_id,
                    reference,
                    auth,
                });
            }
            cx.notify();
        }
    }
}
