//! 容器生命周期与创建、清理动作。

use super::*;

impl AppShell {
    /// 消费拉取/新建弹窗实体上交的提交意图。
    ///
    /// 主输入仍归各标签工作区，实体读不到，因此事件只携带附加字段：外壳先读
    /// `create_input` 做空值校验，失败时写回实体错误并保持弹窗，成功投递命令后
    /// 才关闭实体。事件自带目标标签与分类，处理时禁止回读实体。
    pub(crate) fn on_container_create_event(
        &mut self,
        event: &ContainerCreateDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ContainerCreateDialogEvent::Submit {
            tab_id,
            section,
            driver,
            subnet,
            gateway,
            dockerfile,
            tag,
            internal,
        } = event;
        let Some(workspace) = self.state.container_workspace(*tab_id) else {
            self.close_container_create_dialog(cx);
            return;
        };
        let (value, create_input) = {
            let state = &workspace.read(cx).state;
            (
                state.create_input.read(cx).value().trim().to_string(),
                state.create_input.clone(),
            )
        };
        if value.is_empty() {
            let view = self.state.modals.tools.create_dialog.clone();
            view.update(cx, |view, cx| {
                view.set_error(
                    rust_i18n::t!("container.create_dialog.name_required").to_string(),
                    cx,
                )
            });
            cx.notify();
            return;
        }
        let request_id = ContainerRequestId::new();
        let command = match section {
            ContainerSection::Images => {
                let tags = tag.clone().map(|t| vec![t]).unwrap_or_default();
                ContainerCommand::BuildImage {
                    tab_id: *tab_id,
                    request_id,
                    spec: transom_core::ContainerBuildSpec {
                        context_path: value,
                        dockerfile: dockerfile.clone(),
                        tags,
                        no_cache: false,
                        ..Default::default()
                    },
                }
            }
            ContainerSection::Volumes => ContainerCommand::CreateVolume {
                tab_id: *tab_id,
                request_id,
                name: value,
                driver: driver.clone(),
            },
            ContainerSection::Networks => {
                // 子网/网关/仅内部访问由领域层校验，界面只负责把错误说清楚。
                let mut spec = ContainerNetworkSpec {
                    name: value,
                    driver: driver.clone(),
                    subnet: subnet.clone(),
                    gateway: gateway.clone(),
                    internal: *internal,
                };
                if let Err(error) = spec.validate() {
                    self.show_toast(error, false, window, cx);
                    return;
                }
                ContainerCommand::CreateNetwork {
                    tab_id: *tab_id,
                    request_id,
                    spec,
                }
            }
            _ => return,
        };
        workspace.update(cx, |workspace, _| {
            workspace.state.busy = true;
            workspace.send_tracked(request_id, command);
        });
        create_input.update(cx, |input, cx| input.set_value("", window, cx));
        self.close_container_create_dialog(cx);
    }

    /// 关闭拉取/新建弹窗：实体丢弃本次打开期的附加输入。
    fn close_container_create_dialog(&mut self, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.create_dialog.clone();
        view.update(cx, |view, cx| view.close(cx));
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
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.prune_armed = Some(target);
            });
            cx.notify();
            return;
        }
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.prune_armed = None;
            workspace.busy = true;
        });
        let request_id = ContainerRequestId::new();
        let command = match target {
            ContainerPruneTarget::Volumes => ContainerCommand::PruneVolumes { tab_id, request_id },
            ContainerPruneTarget::Images => ContainerCommand::PruneImages { tab_id, request_id },
            ContainerPruneTarget::Networks => {
                ContainerCommand::PruneNetworks { tab_id, request_id }
            }
        };
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(request_id, command);
            });
        }
        cx.notify();
    }
}

impl ContainerWorkspace {
    /// 生命周期动作（启动/停止/重启/删除）。
    pub(crate) fn container_lifecycle(
        &mut self,
        container: String,
        action: ContainerLifecycleAction,
        cx: &mut Context<Self>,
    ) {
        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.notice =
            Some(rust_i18n::t!("container.action.running", action = action.label()).to_string());
        self.send_tracked(
            request_id,
            ContainerCommand::Lifecycle {
                tab_id: self.tab_id,
                request_id,
                container,
                action,
            },
        );
        cx.notify();
    }

    /// 取消当前分类里的删除确认。
    pub(crate) fn cancel_container_confirm(&mut self, cx: &mut Context<Self>) {
        self.state.confirm = None;
        cx.notify();
    }

    /// 执行确认过的危险操作：把确认项翻译成命令并登记在途请求。
    ///
    /// 设计原理（Why）：确认内容就是本工作区的 `confirm` 快照，命令投递端也归
    /// 实体，确认后的分派不再经外壳转手。Compose 项目先按名称从当前列表取快照，
    /// 找不到说明列表已刷新，本次确认失效；与原外壳行为一致，此时不改弹窗状态。
    pub(crate) fn apply_container_confirm(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.state.confirm.clone() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        let confirm = match confirm {
            ContainerConfirm::Compose(project) => {
                let Some(project) = self
                    .state
                    .compose
                    .iter()
                    .find(|item| item.name == project)
                    .cloned()
                else {
                    return;
                };
                self.send_tracked(
                    request_id,
                    ContainerCommand::RunCompose {
                        tab_id: self.tab_id,
                        request_id,
                        project: Box::new(project),
                        action: transom_core::ComposeAction::Down,
                    },
                );
                cx.notify();
                return;
            }
            confirm => confirm,
        };
        let Some(command) = self.confirm_container_command(confirm, request_id) else {
            return;
        };
        self.state.busy = true;
        self.state.confirm = None;
        self.send_tracked(request_id, command);
        cx.notify();
    }

    /// 把确认项映射成控制器命令；文件类操作缺少选中容器时返回 `None`。
    fn confirm_container_command(
        &self,
        confirm: ContainerConfirm,
        request_id: ContainerRequestId,
    ) -> Option<ContainerCommand> {
        let command = match confirm {
            ContainerConfirm::Container(container) => ContainerCommand::Lifecycle {
                tab_id: self.tab_id,
                request_id,
                container,
                action: ContainerLifecycleAction::Remove { force: false },
            },
            ContainerConfirm::Image(image) => ContainerCommand::RemoveImage {
                tab_id: self.tab_id,
                request_id,
                image,
                force: false,
            },
            ContainerConfirm::Volume(name) => ContainerCommand::RemoveVolume {
                tab_id: self.tab_id,
                request_id,
                name,
                force: false,
            },
            ContainerConfirm::Network(network) => ContainerCommand::RemoveNetwork {
                tab_id: self.tab_id,
                request_id,
                network,
            },
            ContainerConfirm::FilePath { path, recursive } => ContainerCommand::RemovePath {
                tab_id: self.tab_id,
                request_id,
                container: self.state.selected_container_name()?,
                path,
                recursive,
            },
            ContainerConfirm::UploadOverwrite {
                path,
                local_path,
                directory,
            } => {
                let container = self.state.selected_container_name()?;
                if directory {
                    ContainerCommand::UploadDirectory {
                        tab_id: self.tab_id,
                        request_id,
                        container,
                        path,
                        local_path,
                    }
                } else {
                    ContainerCommand::Upload {
                        tab_id: self.tab_id,
                        request_id,
                        container,
                        path,
                        local_path,
                    }
                }
            }
            // Compose 分支在 `apply_container_confirm` 内提前处理。
            ContainerConfirm::Compose(_) => return None,
        };
        Some(command)
    }
}

impl ContainerWorkspace {
    /// 重新发起引擎连接：状态先回到「连接中」，避免占位页继续显示旧错误。
    ///
    /// 设计原理（Why）：重试只依赖本标签页的 profile 与命令通道，属于实体
    /// 自身动作；外壳不再中转，渲染回调直接更新实体即可。
    pub(crate) fn reconnect(&mut self, cx: &mut Context<Self>) {
        let profile = self.state.profile.clone();
        self.send(ContainerCommand::Connect {
            tab_id: self.tab_id,
            profile,
        });
        self.state.status = ContainerWorkspaceStatus::Connecting;
        cx.notify();
    }
}
