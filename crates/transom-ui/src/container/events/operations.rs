//! 清理、动作完成、拉取与系统级结果。

use super::super::*;

impl AppShell {
    pub(super) fn handle_operations_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::VolumesPruned { request_id, report } => {
                self.handle_prune_report(
                    request_id,
                    report.deleted.len(),
                    ContainerPruneTarget::Volumes,
                    window,
                    cx,
                );
            }
            ContainerAppEvent::ImagesPruned { request_id, report } => {
                self.handle_prune_report(
                    request_id,
                    report.deleted.len(),
                    ContainerPruneTarget::Images,
                    window,
                    cx,
                );
            }
            ContainerAppEvent::NetworksPruned { request_id, report } => {
                self.handle_prune_report(
                    request_id,
                    report.deleted.len(),
                    ContainerPruneTarget::Networks,
                    window,
                    cx,
                );
            }
            ContainerAppEvent::ActionFinished { request_id, error } => {
                self.on_container_action_finished(request_id, error, window, cx);
            }
            ContainerAppEvent::TransferFinished { request_id, error } => {
                self.on_container_transfer_finished(request_id, error, window, cx);
            }
            ContainerAppEvent::Failed { request_id, error } => {
                self.on_container_failed(request_id, error, window, cx);
            }
            ContainerAppEvent::ImagesSearched {
                request_id,
                results,
            } => {
                self.on_container_images_searched(request_id, results, window, cx);
            }
            ContainerAppEvent::ImageTagsListed { request_id, tags } => {
                self.on_container_image_tags_listed(request_id, tags, cx);
            }
            ContainerAppEvent::PullTaskProgress {
                task_id,
                status,
                layers,
            } => {
                self.on_container_pull_task_progress(task_id, status, layers, window, cx);
            }
            ContainerAppEvent::PullTaskFinished { task_id, error } => {
                self.on_container_pull_task_finished(task_id, error, window, cx);
            }
            ContainerAppEvent::ContainerDiffed {
                request_id,
                container,
                changes,
            } => {
                self.on_container_diffed(request_id, container, changes);
            }
            ContainerAppEvent::SystemDfLoaded { request_id, usage } => {
                self.on_container_system_df_loaded(request_id, usage);
            }
            ContainerAppEvent::SystemPruned { request_id, report } => {
                self.on_container_system_pruned(request_id, report, window, cx);
            }
            ContainerAppEvent::EngineEventReceived { tab_id, event } => {
                self.on_container_engine_event_received(tab_id, event, window, cx);
            }
            _ => {}
        }
    }
}
