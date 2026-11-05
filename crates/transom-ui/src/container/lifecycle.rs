//! 容器生命周期与创建、清理动作。

use super::*;

impl AppShell {
    /// 取消当前分类里的删除确认。
    pub(crate) fn cancel_container_confirm(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.confirm = None;
        }
        cx.notify();
    }

    /// 生命周期动作（启动/停止/重启/删除）。
    pub(crate) fn container_lifecycle(
        &mut self,
        tab_id: TabId,
        container: String,
        action: ContainerLifecycleAction,
        cx: &mut Context<Self>,
    ) {
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.notice = Some(
                rust_i18n::t!("container.action.running", action = action.label()).to_string(),
            );
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Lifecycle {
                tab_id,
                request_id,
                container,
                action,
            },
        );
        cx.notify();
    }

    /// 更新容器资源配置（内存、CPU、重启策略）。
    pub(crate) fn update_container_resources(
        &mut self,
        tab_id: TabId,
        container: String,
        spec: transom_core::ContainerUpdateSpec,
        cx: &mut Context<Self>,
    ) {
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.notice = Some(
                rust_i18n::t!(
                    "container.action.running",
                    action = rust_i18n::t!("container.action.update")
                )
                .to_string(),
            );
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::UpdateContainer {
                tab_id,
                request_id,
                container,
                spec,
            },
        );
        cx.notify();
    }

    /// 删除镜像/卷/网络。
    pub(crate) fn apply_container_confirm(
        &mut self,
        tab_id: TabId,
        confirm: ContainerConfirm,
        cx: &mut Context<Self>,
    ) {
        let request_id = ContainerRequestId::new();
        let command = match confirm {
            ContainerConfirm::Compose(project) => {
                let Some(project) = self
                    .state
                    .container_workspace(tab_id)
                    .and_then(|workspace| {
                        workspace
                            .compose
                            .iter()
                            .find(|item| item.name == project)
                            .cloned()
                    })
                else {
                    return;
                };
                self.send_tracked(
                    tab_id,
                    request_id,
                    ContainerCommand::RunCompose {
                        tab_id,
                        request_id,
                        project: Box::new(project),
                        action: transom_core::ComposeAction::Down,
                    },
                );
                cx.notify();
                return;
            }
            ContainerConfirm::Container(container) => ContainerCommand::Lifecycle {
                tab_id,
                request_id,
                container,
                action: ContainerLifecycleAction::Remove { force: false },
            },
            ContainerConfirm::Image(image) => ContainerCommand::RemoveImage {
                tab_id,
                request_id,
                image,
                force: false,
            },
            ContainerConfirm::Volume(name) => ContainerCommand::RemoveVolume {
                tab_id,
                request_id,
                name,
                force: false,
            },
            ContainerConfirm::Network(network) => ContainerCommand::RemoveNetwork {
                tab_id,
                request_id,
                network,
            },
            ContainerConfirm::FilePath { path, recursive } => {
                let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
                    return;
                };
                let Some(container) = workspace.selected_container_name() else {
                    return;
                };
                ContainerCommand::RemovePath {
                    tab_id,
                    request_id,
                    container,
                    path,
                    recursive,
                }
            }
        };
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.confirm = None;
        }
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }

    /// 创建卷/网络，或拉取镜像（按当前分类解释输入框）。
    pub(crate) fn submit_container_create(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 附加输入都挂在弹窗状态上，取走它们必须在借用工作区之前。
        let dialog = self.state.containers.create_dialog.as_ref();
        let read_input = |input: &Option<Entity<InputState>>| {
            input
                .as_ref()
                .map(|input| input.read(cx).value().trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let driver = read_input(&dialog.and_then(|dialog| dialog.driver.clone()));
        let subnet = read_input(&dialog.and_then(|dialog| dialog.subnet.clone()));
        let gateway = read_input(&dialog.and_then(|dialog| dialog.gateway.clone()));
        let dockerfile = read_input(&dialog.and_then(|dialog| dialog.dockerfile.clone()));
        let tag = read_input(&dialog.and_then(|dialog| dialog.tag.clone()));
        let internal = dialog.is_some_and(|dialog| dialog.internal);
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let section = workspace.section;
        let value = workspace.create_input.read(cx).value().trim().to_string();
        if value.is_empty() {
            self.show_toast(
                rust_i18n::t!("container.create_dialog.name_required").to_string(),
                false,
                window,
                cx,
            );
            return;
        }
        let request_id = ContainerRequestId::new();
        let command = match section {
            ContainerSection::Images => {
                let tags = tag.map(|t| vec![t]).unwrap_or_default();
                ContainerCommand::BuildImage {
                    tab_id,
                    request_id,
                    spec: transom_core::ContainerBuildSpec {
                        context_path: value,
                        dockerfile,
                        tags,
                        no_cache: false,
                    },
                }
            }
            ContainerSection::Volumes => ContainerCommand::CreateVolume {
                tab_id,
                request_id,
                name: value,
                driver,
            },
            ContainerSection::Networks => {
                // 子网/网关/仅内部访问由领域层校验，界面只负责把错误说清楚。
                let mut spec = ContainerNetworkSpec {
                    name: value,
                    driver,
                    subnet,
                    gateway,
                    internal,
                };
                if let Err(error) = spec.validate() {
                    self.show_toast(error, false, window, cx);
                    return;
                }
                ContainerCommand::CreateNetwork {
                    tab_id,
                    request_id,
                    spec,
                }
            }
            _ => return,
        };
        workspace.busy = true;
        workspace
            .create_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.containers.create_dialog = None;
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }

    /// 打开拉取/新建弹窗：镜像只有一个引用输入，卷与网络多一个驱动输入，
    /// 网络再加子网、网关与「仅内部访问」。
    pub(crate) fn open_container_create_dialog(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let driver = matches!(
            section,
            ContainerSection::Volumes | ContainerSection::Networks
        )
        .then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.driver_placeholder").to_string(),
                )
            })
        });
        let network = section == ContainerSection::Networks;
        let subnet = network.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.subnet_placeholder").to_string(),
                )
            })
        });
        let gateway = network.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.gateway_placeholder").to_string(),
                )
            })
        });
        let is_image = section == ContainerSection::Images;
        let dockerfile = is_image.then(|| {
            cx.new(|cx| InputState::new(window, cx).placeholder("Dockerfile".to_string()))
        });
        let tag = is_image.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    rust_i18n::t!("container.create_dialog.tag_placeholder").to_string(),
                )
            })
        });
        self.state.containers.create_dialog = Some(ContainerCreateDialog {
            tab_id,
            section,
            driver,
            subnet,
            gateway,
            dockerfile,
            tag,
            internal: false,
        });
        cx.notify();
    }

    pub(crate) fn dismiss_container_create(&mut self, cx: &mut Context<Self>) {
        self.state.containers.create_dialog = None;
        cx.notify();
    }

    /// 清理未使用卷或悬空镜像：删除不可恢复，因此第一下只待确认，第二下才下发。
    ///
    /// 两个目标的确认状态共用 `prune_armed` 一个槽位：同一时刻只会有一个待确认
    /// 动作，分开存反而可能出现两条按钮同时武装。
    pub(crate) fn prune_container_target(
        &mut self,
        tab_id: TabId,
        target: ContainerPruneTarget,
        armed: bool,
        cx: &mut Context<Self>,
    ) {
        if !armed {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.prune_armed = Some(target);
            }
            cx.notify();
            return;
        }
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.prune_armed = None;
        workspace.busy = true;
        let request_id = ContainerRequestId::new();
        let command = match target {
            ContainerPruneTarget::Volumes => ContainerCommand::PruneVolumes { tab_id, request_id },
            ContainerPruneTarget::Images => ContainerCommand::PruneImages { tab_id, request_id },
            ContainerPruneTarget::Networks => {
                ContainerCommand::PruneNetworks { tab_id, request_id }
            }
        };
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }
}
