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
        let tab_id = self.state.containers.take_tab(request_id);
        if let Some(tab_id) = tab_id {
            let applied = self.with_container_mut(tab_id, cx, |workspace| {
                let Some(hub) = workspace.hub_dialog.as_mut() else {
                    return false;
                };
                if hub.search_request != Some(request_id) {
                    return false;
                }
                hub.searching = false;
                hub.search_request = None;
                hub.results = results;
                hub.search_error = None;
                true
            });
            if applied == Some(true) {
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
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
            return;
        };
        let applied = self.with_container_mut(tab_id, cx, |workspace| {
            let Some(hub) = workspace.hub_dialog.as_mut() else {
                return false;
            };
            if hub.tags_request != Some(request_id) {
                return false;
            }
            hub.tags = tags;
            hub.tags_loading = false;
            hub.tags_error = None;
            hub.tags_request = None;
            // 若当前选中的 tag 不在实时列表里（仓库已删除该 tag），回落到 latest。
            if let Some(detail) = hub.selected_detail.as_ref() {
                let selected = hub
                    .selected_tags
                    .entry(detail.name.clone())
                    .or_insert_with(|| "latest".to_string());
                if !hub.tags.is_empty() && !hub.tags.iter().any(|tag| tag == selected) {
                    *selected = hub.tags[0].clone();
                }
            }
            true
        });
        if applied == Some(true) {
            cx.notify();
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
            self.with_container(*tab_id, cx, |workspace| {
                workspace.pull_manager.get(&task_id).is_some()
            })
            .unwrap_or(false)
        });
        if let Some(tab_id) = target {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.pull_manager.on_progress(&task_id, status, layers);
            });
            cx.notify();
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
            let found = self
                .with_container_mut(tab_id, cx, |workspace| {
                    let task = workspace.pull_manager.get(&task_id)?;
                    let reference = task.reference.clone();
                    let next = workspace
                        .pull_manager
                        .on_finished(&task_id, error_msg.clone());
                    Some((reference, next))
                })
                .flatten();
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
                self.send_tracked(
                    tab_id,
                    req_images,
                    ContainerCommand::ListImages {
                        tab_id,
                        request_id: req_images,
                    },
                );
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
