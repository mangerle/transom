//! 容器工作区延迟请求处理。

use super::AppShell;
use gpui_kit::component::input::TextareaState;
use gpui_kit::*;

impl AppShell {
    /// 执行一条在事件派发期不能直接完成的容器工作区动作。
    ///
    /// 设计原理（Why）：事件在标签实体派发阶段产生，那里既拿不到 `Window`，
    /// 也不能在实体正被租借时回读它；外壳先把事件连同订阅捕获的标签 id 入队，
    /// 渲染期再调用既有打开/切换方法。这里只处理需要延后的分支，纯实体动作已
    /// 在事件处理函数之外由工作区自己完成。
    pub(super) fn handle_container_request(
        &mut self,
        tab_id: transom_core::TabId,
        event: &crate::container::ContainerWorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::container::ContainerDetailTab;
        use crate::container::ContainerWorkspaceEvent;
        match event {
            ContainerWorkspaceEvent::OpenRunForm(image) => {
                crate::container::open_container_run_form(self, tab_id, image.clone(), window, cx);
            }
            ContainerWorkspaceEvent::OpenImageHub => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_hub_dialog(window, cx);
                    });
                }
            }
            ContainerWorkspaceEvent::OpenCreateDialog(section) => {
                let view = self.state.modals.tools.create_dialog.clone();
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
                    self, tab_id, container, *private, *public, window, cx,
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
                        workspace.state.compose_yaml_editor =
                            Some((project.clone(), path.clone(), editor, false));
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
                crate::container::clone_container(self, tab_id, id, name, window, cx);
            }
            ContainerWorkspaceEvent::RecreateContainer { id, name } => {
                crate::container::recreate_container(self, tab_id, id, name, window, cx);
            }
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
                                self,
                                tab_id,
                                Some(image),
                                window,
                                cx,
                            );
                        }
                        crate::container::ShellRowAction::CloneContainer { id, name } => {
                            crate::container::clone_container(self, tab_id, &id, &name, window, cx);
                        }
                        crate::container::ShellRowAction::EditComposeYaml(name) => {
                            crate::container::edit_compose_yaml(self, tab_id, &name, window, cx);
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
