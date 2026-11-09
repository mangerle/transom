//! 连接、发现与测试连接结果。

use super::super::*;

impl AppShell {
    pub(super) fn handle_connection_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::Connected { tab_id, info } => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.status = ContainerWorkspaceStatus::Connected;
                        workspace.state.info = Some(*info);
                        workspace.state.notice = None;
                    });
                }
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Connected;
                }
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    let workspace = &workspace.read(cx).state;
                    let profile = workspace.profile.clone();
                    let mut exists = false;
                    for saved in &mut self.state.catalog.engines {
                        if saved.id == profile.id
                            || (saved.engine.kind == profile.engine.kind
                                && saved.engine.endpoint.describe()
                                    == profile.engine.endpoint.describe())
                        {
                            exists = true;
                            if saved.engine.endpoint != profile.engine.endpoint {
                                saved.engine.endpoint = profile.engine.endpoint.clone();
                                saved.touch();
                                self.state.persist_catalog();
                                cx.notify();
                            }
                            break;
                        }
                    }
                    if !exists {
                        self.state.catalog.engines.push(profile);
                        self.state.persist_catalog();
                        cx.notify();
                    }
                }
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| workspace.refresh_all());
                }
            }
            ContainerAppEvent::ConnectFailed { tab_id, error } => {
                let message = localized_error(&error);
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.status = ContainerWorkspaceStatus::Failed(message.clone());
                    });
                }
                // 连接失败既要让工作区停在失败态（重试按钮在那里），也要在右上角
                // 立刻说一声：标签可能不在前台，只靠面板状态用户看不到出了事。
                self.show_toast(message, false, window, cx);
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Failed(error);
                }
            }
            ContainerAppEvent::EnginesDiscovered {
                engines, failures, ..
            } => {
                let sidebar = self.state.containers.sidebar.clone();
                sidebar.update(cx, |sidebar, cx| {
                    sidebar.apply_discovery(
                        ContainerDiscovery {
                            candidates: engines,
                            failures,
                        },
                        cx,
                    );
                });
            }
            ContainerAppEvent::TestFinished {
                request_id,
                info,
                error,
            } => {
                let form = self.state.modals.tools.container_form.clone();
                form.update(cx, |form, cx| {
                    form.set_test_result(request_id, info, error, cx);
                });
            }
            ContainerAppEvent::Disconnected { tab_id } => {
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.status = ContainerWorkspaceStatus::Disconnected;
                    });
                }
            }
            _ => {}
        }
    }
}
