//! 容器/镜像/卷/网络/编排等列表与详情结果。
//!
//! 设计原理（Why）：事件处理只按 `request_id` 定位标签页，具体状态流转
//! 收敛到 [`ContainerWorkspace`] 的具名方法，避免在外壳闭包里堆业务逻辑。

use super::super::*;

impl AppShell {
    pub(super) fn handle_listing_event(
        &mut self,
        event: ContainerAppEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::ContainersListed {
                request_id,
                containers,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_containers_listed(containers)
                    });
                }
            }
            ContainerAppEvent::ContainerInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_container_inspected(*details)
                    });
                }
            }
            ContainerAppEvent::ImagesListed { request_id, images } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| workspace.apply_images_listed(images));
                }
            }
            ContainerAppEvent::ImageInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| workspace.apply_image_inspected(*details));
                }
            }
            ContainerAppEvent::VolumesListed {
                request_id,
                volumes,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| workspace.apply_volumes_listed(volumes));
                }
            }
            ContainerAppEvent::VolumeInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_volume_inspected(*details)
                    });
                }
            }
            ContainerAppEvent::NetworksListed {
                request_id,
                networks,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| workspace.apply_networks_listed(networks));
                }
            }
            ContainerAppEvent::NetworkInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_network_inspected(*details)
                    });
                }
            }
            ContainerAppEvent::ComposeProjectsListed {
                request_id,
                projects,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_compose_projects_listed(projects)
                    });
                }
            }
            ContainerAppEvent::PodsListed { request_id, pods } => {
                self.on_container_pods_listed(request_id, pods, cx);
            }
            ContainerAppEvent::QuadletsListed { request_id, units } => {
                self.on_container_quadlets_listed(request_id, units, cx);
            }
            ContainerAppEvent::ProcessesListed {
                request_id,
                processes,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_processes_listed(processes)
                    });
                }
            }
            _ => {}
        }
    }
}
