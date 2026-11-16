//! 清理、动作完成、拉取与系统级结果。

use super::super::*;

pub(super) fn handle_operations_event(
    app_state: &mut AppState,
    event: ContainerAppEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        ContainerAppEvent::VolumesPruned { request_id, report } => {
            super::result::handle_prune_report(
                app_state,
                request_id,
                report.deleted.len(),
                ContainerPruneTarget::Volumes,
                window,
                cx,
            );
        }
        ContainerAppEvent::ImagesPruned { request_id, report } => {
            super::result::handle_prune_report(
                app_state,
                request_id,
                report.deleted.len(),
                ContainerPruneTarget::Images,
                window,
                cx,
            );
        }
        ContainerAppEvent::NetworksPruned { request_id, report } => {
            super::result::handle_prune_report(
                app_state,
                request_id,
                report.deleted.len(),
                ContainerPruneTarget::Networks,
                window,
                cx,
            );
        }
        ContainerAppEvent::ActionFinished { request_id, error } => {
            super::result::on_container_action_finished(app_state, request_id, error, window, cx);
        }
        ContainerAppEvent::TransferFinished { request_id, error } => {
            super::result::on_container_transfer_finished(app_state, request_id, error, window, cx);
        }
        ContainerAppEvent::Failed { request_id, error } => {
            super::result::on_container_failed(app_state, request_id, error, window, cx);
        }
        ContainerAppEvent::ImagesSearched {
            request_id,
            results,
        } => {
            super::pull::on_container_images_searched(app_state, request_id, results, window, cx);
        }
        ContainerAppEvent::ImageTagsListed { request_id, tags } => {
            super::pull::on_container_image_tags_listed(app_state, request_id, tags, cx);
        }
        ContainerAppEvent::PullTaskProgress {
            task_id,
            status,
            layers,
        } => {
            super::pull::on_container_pull_task_progress(
                app_state, task_id, status, layers, window, cx,
            );
        }
        ContainerAppEvent::PullTaskFinished { task_id, error } => {
            super::pull::on_container_pull_task_finished(app_state, task_id, error, window, cx);
        }
        ContainerAppEvent::ContainerDiffed {
            request_id,
            container,
            changes,
        } => {
            super::system::on_container_diffed(app_state, request_id, container, changes, cx);
        }
        ContainerAppEvent::SystemDfLoaded { request_id, usage } => {
            super::system::on_container_system_df_loaded(app_state, request_id, usage, cx);
        }
        ContainerAppEvent::SystemPruned { request_id, report } => {
            super::system::on_container_system_pruned(app_state, request_id, report, window, cx);
        }
        ContainerAppEvent::EngineEventReceived { tab_id, event } => {
            super::system::on_container_engine_event_received(app_state, tab_id, event, window, cx);
        }
        _ => {}
    }
}
