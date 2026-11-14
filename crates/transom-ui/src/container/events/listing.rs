//! 容器/镜像/卷/网络/编排等列表与详情结果。
//!
//! 设计原理（Why）：事件处理只按 `request_id` 定位标签页，具体状态流转
//! 收敛到 [`ContainerWorkspace`] 的具名方法，避免在外壳闭包里堆业务逻辑。

use super::super::*;

fn apply_by_request<F>(
    shell: &mut AppShell,
    request_id: ContainerRequestId,
    cx: &mut Context<AppShell>,
    apply: F,
) where
    F: FnOnce(&mut ContainerWorkspace),
{
    let Some(tab_id) = shell.state.take_container_request(request_id, cx) else {
        return;
    };
    if let Some(workspace) = shell.state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, _| {
            apply(workspace);
        });
    }
}

pub(super) fn handle_listing_event(
    shell: &mut AppShell,
    event: ContainerAppEvent,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        ContainerAppEvent::ContainersListed {
            request_id,
            containers,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_containers_listed(containers)
        }),
        ContainerAppEvent::ContainerInspected {
            request_id,
            details,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_container_inspected(*details)
        }),
        ContainerAppEvent::ImagesListed { request_id, images } => {
            apply_by_request(shell, request_id, cx, |w| w.apply_images_listed(images))
        }
        ContainerAppEvent::ImageInspected {
            request_id,
            details,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_image_inspected(*details)
        }),
        ContainerAppEvent::VolumesListed {
            request_id,
            volumes,
        } => apply_by_request(shell, request_id, cx, |w| w.apply_volumes_listed(volumes)),
        ContainerAppEvent::VolumeInspected {
            request_id,
            details,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_volume_inspected(*details)
        }),
        ContainerAppEvent::NetworksListed {
            request_id,
            networks,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_networks_listed(networks)
        }),
        ContainerAppEvent::NetworkInspected {
            request_id,
            details,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_network_inspected(*details)
        }),
        ContainerAppEvent::ComposeProjectsListed {
            request_id,
            projects,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_compose_projects_listed(projects)
        }),
        ContainerAppEvent::PodsListed { request_id, pods } => {
            apply_by_request(shell, request_id, cx, |w| {
                w.apply_pods_listed(request_id, pods);
            });
        }
        ContainerAppEvent::QuadletsListed { request_id, units } => {
            apply_by_request(shell, request_id, cx, |w| {
                w.apply_quadlets_listed(request_id, units);
            });
        }
        ContainerAppEvent::ProcessesListed {
            request_id,
            processes,
        } => apply_by_request(shell, request_id, cx, |w| {
            w.apply_processes_listed(processes)
        }),
        _ => {}
    }
}
