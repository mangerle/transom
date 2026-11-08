//! 容器/镜像/卷/网络/编排等列表与详情结果。

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
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                let mut inspect_first = None;
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.compose = transom_core::compose_projects(&containers);
                    workspace.containers = containers;
                    workspace.busy = false;
                    // 只有容器/编排分类的选中项是容器 ID；在卷等其他分类下顺手
                    // 拉一次容器列表时，不能把当前的选中项判成"无效"清掉。
                    let id_is_container_scoped = matches!(
                        workspace.section,
                        ContainerSection::Containers | ContainerSection::Compose
                    );
                    if id_is_container_scoped {
                        let valid = workspace.selected.as_deref().is_some_and(|id| {
                            workspace.containers.iter().any(|item| item.id == id)
                        });
                        if !valid {
                            workspace.selected = None;
                            workspace.details = None;
                        }
                    }
                    // 跨分类跳转等着列表到位再按名字定位，避免用户点了两次。
                    if let Some(name) = workspace.pending_select.take() {
                        inspect_first = workspace
                            .containers
                            .iter()
                            .find(|item| item.name == name)
                            .map(|item| item.id.clone());
                    }
                    if workspace.section == ContainerSection::Containers
                        && workspace.selected.is_none()
                        && inspect_first.is_none()
                        && let Some(first) = workspace.containers.first()
                    {
                        inspect_first = Some(first.id.clone());
                    }
                });
                let re_inspect_id = if inspect_first.is_none() {
                    self.with_container(tab_id, cx, |workspace| workspace.selected.clone())
                        .flatten()
                } else {
                    None
                };
                if let Some(id) = inspect_first {
                    self.select_container_item(tab_id, id, cx);
                } else if let Some(id) = re_inspect_id {
                    let request_id = ContainerRequestId::new();
                    if let Some(workspace) = self.state.container_workspace(tab_id) {
                        workspace.update(cx, |workspace, _| {
                            workspace.send_tracked(
                                request_id,
                                ContainerCommand::InspectContainer {
                                    tab_id,
                                    request_id,
                                    container: id,
                                },
                            );
                        });
                    }
                }
            }
            ContainerAppEvent::ContainerInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.selected = Some(details.id.clone());
                    workspace.details = Some(*details);
                });
            }
            ContainerAppEvent::ImagesListed { request_id, images } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                let selected = self
                    .with_container_mut(tab_id, cx, |workspace| {
                        workspace.images = images;
                        workspace.busy = false;
                        let valid = workspace
                            .selected
                            .as_deref()
                            .is_some_and(|id| workspace.images.iter().any(|item| item.id == id));
                        if !valid {
                            workspace.selected =
                                workspace.images.first().map(|item| item.id.clone());
                            workspace.image_details = None;
                        }
                        workspace.selected.clone()
                    })
                    .flatten();
                if let Some(id) = selected {
                    let request_id = ContainerRequestId::new();
                    if let Some(workspace) = self.state.container_workspace(tab_id) {
                        workspace.update(cx, |workspace, _| {
                            workspace.send_tracked(
                                request_id,
                                ContainerCommand::InspectImage {
                                    tab_id,
                                    request_id,
                                    image: id,
                                },
                            );
                        });
                    }
                }
            }
            ContainerAppEvent::ImageInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.selected = Some(details.id.clone());
                    workspace.image_details = Some(*details);
                });
            }
            ContainerAppEvent::VolumesListed {
                request_id,
                volumes,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                let selected = self
                    .with_container_mut(tab_id, cx, |workspace| {
                        workspace.volumes = volumes;
                        workspace.busy = false;
                        let valid = workspace.selected.as_deref().is_some_and(|name| {
                            workspace.volumes.iter().any(|item| item.name == name)
                        });
                        if !valid {
                            workspace.selected =
                                workspace.volumes.first().map(|item| item.name.clone());
                            workspace.volume_details = None;
                        }
                        workspace.selected.clone()
                    })
                    .flatten();
                if let Some(name) = selected {
                    let request_id = ContainerRequestId::new();
                    if let Some(workspace) = self.state.container_workspace(tab_id) {
                        workspace.update(cx, |workspace, _| {
                            workspace.send_tracked(
                                request_id,
                                ContainerCommand::InspectVolume {
                                    tab_id,
                                    request_id,
                                    name,
                                },
                            );
                        });
                    }
                }
            }
            ContainerAppEvent::VolumeInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.selected = Some(details.summary.name.clone());
                    workspace.volume_details = Some(*details);
                });
            }
            ContainerAppEvent::NetworksListed {
                request_id,
                networks,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                let selected = self
                    .with_container_mut(tab_id, cx, |workspace| {
                        workspace.networks = networks;
                        workspace.busy = false;
                        let valid = workspace
                            .selected
                            .as_deref()
                            .is_some_and(|id| workspace.networks.iter().any(|item| item.id == id));
                        if !valid {
                            workspace.selected =
                                workspace.networks.first().map(|item| item.id.clone());
                            workspace.network_details = None;
                        }
                        workspace.selected.clone()
                    })
                    .flatten();
                if let Some(id) = selected {
                    let request_id = ContainerRequestId::new();
                    if let Some(workspace) = self.state.container_workspace(tab_id) {
                        workspace.update(cx, |workspace, _| {
                            workspace.send_tracked(
                                request_id,
                                ContainerCommand::InspectNetwork {
                                    tab_id,
                                    request_id,
                                    network: id,
                                },
                            );
                        });
                    }
                }
            }
            ContainerAppEvent::NetworkInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.selected = Some(details.summary.id.clone());
                    workspace.network_details = Some(*details);
                });
            }
            ContainerAppEvent::ComposeProjectsListed {
                request_id,
                projects,
            } => {
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.compose = projects;
                    workspace.busy = false;
                });
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
                let Some(tab_id) = self.take_container_request(request_id, cx) else {
                    return;
                };
                self.with_container_mut(tab_id, cx, |workspace| {
                    workspace.stats_processes_request = None;
                    workspace.stats_processes_loading = false;
                    workspace.stats_processes = processes;
                    workspace.stats_processes_error = None;
                });
            }
            _ => {}
        }
    }
}
