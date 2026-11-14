//! 清理、动作完成、拉取与系统级结果。

use super::super::*;

pub(super) fn handle_operations_event(
    shell: &mut AppShell,
    event: ContainerAppEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        ContainerAppEvent::VolumesPruned { request_id, report } => {
            super::result::handle_prune_report(
                shell,
                request_id,
                report.deleted.len(),
                ContainerPruneTarget::Volumes,
                window,
                cx,
            );
        }
        ContainerAppEvent::ImagesPruned { request_id, report } => {
            super::result::handle_prune_report(
                shell,
                request_id,
                report.deleted.len(),
                ContainerPruneTarget::Images,
                window,
                cx,
            );
        }
        ContainerAppEvent::NetworksPruned { request_id, report } => {
            super::result::handle_prune_report(
                shell,
                request_id,
                report.deleted.len(),
                ContainerPruneTarget::Networks,
                window,
                cx,
            );
        }
        ContainerAppEvent::ActionFinished { request_id, error } => {
            super::result::on_container_action_finished(shell, request_id, error, window, cx);
        }
        ContainerAppEvent::TransferFinished { request_id, error } => {
            super::result::on_container_transfer_finished(shell, request_id, error, window, cx);
        }
        ContainerAppEvent::Failed { request_id, error } => {
            super::result::on_container_failed(shell, request_id, error, window, cx);
        }
        ContainerAppEvent::ImagesSearched {
            request_id,
            results,
        } => {
            super::pull::on_container_images_searched(shell, request_id, results, window, cx);
        }
        ContainerAppEvent::ImageTagsListed { request_id, tags } => {
            super::pull::on_container_image_tags_listed(shell, request_id, tags, cx);
        }
        ContainerAppEvent::PullTaskProgress {
            task_id,
            status,
            layers,
        } => {
            super::pull::on_container_pull_task_progress(
                shell, task_id, status, layers, window, cx,
            );
        }
        ContainerAppEvent::PullTaskFinished { task_id, error } => {
            super::pull::on_container_pull_task_finished(shell, task_id, error, window, cx);
        }
        ContainerAppEvent::ContainerDiffed {
            request_id,
            container,
            changes,
        } => {
            super::system::on_container_diffed(shell, request_id, container, changes, cx);
        }
        ContainerAppEvent::SystemDfLoaded { request_id, usage } => {
            super::system::on_container_system_df_loaded(shell, request_id, usage, cx);
        }
        ContainerAppEvent::SystemPruned { request_id, report } => {
            super::system::on_container_system_pruned(shell, request_id, report, window, cx);
        }
        ContainerAppEvent::EngineEventReceived { tab_id, event } => {
            super::system::on_container_engine_event_received(shell, tab_id, event, window, cx);
        }
        _ => {}
    }
}
