//! 容器工作区延迟请求处理。

use super::AppShell;
use crate::container::{ContainerDetailTab, ContainerWorkspaceEvent};
use gpui_kit::component::input::TextareaState;
use gpui_kit::*;

impl AppShell {
    /// 执行一条在事件派发期不能直接完成的容器工作区动作。
    pub(super) fn handle_container_request(
        &mut self,
        tab_id: transom_core::TabId,
        event: &ContainerWorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            event,
            ContainerWorkspaceEvent::OpenRunForm(_)
                | ContainerWorkspaceEvent::OpenImageHub
                | ContainerWorkspaceEvent::OpenCreateDialog(_)
                | ContainerWorkspaceEvent::OpenBuildDialog
                | ContainerWorkspaceEvent::OpenPortForward { .. }
                | ContainerWorkspaceEvent::OpenVolumeFiles
                | ContainerWorkspaceEvent::ComposeYamlLoaded { .. }
                | ContainerWorkspaceEvent::ComposeYamlReadFailed(_)
                | ContainerWorkspaceEvent::OpenContainerUpdate(_)
                | ContainerWorkspaceEvent::OpenContainerExport(_)
                | ContainerWorkspaceEvent::OpenContainerCommit(_)
                | ContainerWorkspaceEvent::OpenImageTag(_)
                | ContainerWorkspaceEvent::OpenImagePush { .. }
                | ContainerWorkspaceEvent::OpenNetworkConnect(_)
                | ContainerWorkspaceEvent::CloneContainer { .. }
                | ContainerWorkspaceEvent::RecreateContainer { .. }
        ) {
            self.handle_container_modal_request(tab_id, event, window, cx);
            return;
        }
        self.handle_container_action_request(tab_id, event, window, cx);
    }

    /// 打开或装配容器工作区弹窗、编辑器与派生表单。
    fn handle_container_modal_request(
        &mut self,
        tab_id: transom_core::TabId,
        event: &ContainerWorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerWorkspaceEvent::OpenRunForm(image) => {
                crate::container::open_container_run_form(
                    &mut self.state,
                    tab_id,
                    image.clone(),
                    window,
                    cx,
                );
            }
            ContainerWorkspaceEvent::OpenImageHub => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_hub_dialog(window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenCreateDialog(section) => {
                let view = self.state.ui.modals.tools.create_dialog.clone();
                view.update(cx, |view, cx| view.open(tab_id, *section, window, cx));
                cx.notify();
            }
            ContainerWorkspaceEvent::OpenBuildDialog => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_build_dialog(window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenPortForward {
                container,
                private,
                public,
            } => {
                crate::container::open_container_port_forward(
                    &mut self.state,
                    crate::container::ContainerPortForwardOpen {
                        tab_id,
                        container_name: container,
                        private_port: *private,
                        published_port: *public,
                    },
                    window,
                    cx,
                );
            }
            ContainerWorkspaceEvent::OpenVolumeFiles => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.state.detail_tab = ContainerDetailTab::Files;
                        workspace.load_container_dir("/".to_string(), cx);
                    });
                }
            }
            ContainerWorkspaceEvent::ComposeYamlLoaded {
                project,
                path,
                text,
            } => {
                let Some(workspace) = self.state.container_workspace(tab_id) else {
                    return;
                };
                let editor =
                    cx.new(|cx| TextareaState::new(window, cx).default_value(text.clone()));
                workspace.update(cx, |workspace, cx| {
                    // 读取期间用户可能已切到别的项目：只把结果写给仍被选中的项目。
                    if workspace
                        .state
                        .selected_compose()
                        .is_some_and(|item| &item.name == project)
                    {
                        workspace.state.compose_yaml_subscription = None;
                        let subscription = cx.observe(&editor, |_, _, cx| cx.notify());
                        workspace.state.compose_yaml_editor =
                            Some((project.clone(), path.clone(), editor, false));
                        workspace.state.compose_yaml_subscription = Some(subscription);
                        cx.notify();
                    }
                });
            }
            ContainerWorkspaceEvent::ComposeYamlReadFailed(message) => {
                self.show_toast(message.clone(), false, window, cx);
            }
            ContainerWorkspaceEvent::OpenContainerUpdate(name) => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_container_update(name.clone(), window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenContainerExport(name) => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_container_export(name.clone(), cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenContainerCommit(name) => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_container_commit(name.clone(), window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenImageTag(image) => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_image_tag(image.clone(), window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenImagePush { id, reference } => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_push_dialog(id.clone(), reference.clone(), window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenNetworkConnect(network) => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_network_connect(network.clone(), window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::CloneContainer { id, name } => {
                crate::container::clone_container(&mut self.state, tab_id, id, name, window, cx);
            }
            ContainerWorkspaceEvent::RecreateContainer { id, name } => {
                crate::container::recreate_container(&mut self.state, tab_id, id, name, window, cx);
            }
            _ => {}
        }
    }

    /// 处理详情页页签切换、行动作与提示。
    fn handle_container_action_request(
        &mut self,
        tab_id: transom_core::TabId,
        event: &ContainerWorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerWorkspaceEvent::SetDetailTab(tab) => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| workspace.set_detail_tab(*tab, cx));
                }
            }
            ContainerWorkspaceEvent::RunRowAction { target, action } => {
                let Some(workspace) = self.state.container_workspace(tab_id) else {
                    return;
                };
                let pending = workspace.update(cx, |workspace, cx| {
                    workspace.apply_row_action(target.clone(), *action, window, cx)
                });
                if let Some(pending) = pending {
                    match pending {
                        crate::container::ShellRowAction::RunImage(image) => {
                            crate::container::open_container_run_form(
                                &mut self.state,
                                tab_id,
                                Some(image),
                                window,
                                cx,
                            );
                        }
                        crate::container::ShellRowAction::CloneContainer { id, name } => {
                            crate::container::clone_container(
                                &mut self.state,
                                tab_id,
                                &id,
                                &name,
                                window,
                                cx,
                            );
                        }
                    }
                }
            }
            ContainerWorkspaceEvent::ShowToast { message, success } => {
                self.show_toast(message.clone(), *success, window, cx);
            }
            _ => {}
        }
    }
}
