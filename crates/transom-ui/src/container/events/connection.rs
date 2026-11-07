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
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Connected;
                    workspace.info = Some(*info);
                    workspace.notice = None;
                }
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Connected;
                }
                if let Some(workspace) = self.state.container_workspace(tab_id) {
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
                self.refresh_container_workspace_all(tab_id);
            }
            ContainerAppEvent::ConnectFailed { tab_id, error } => {
                let message = localized_error(&error);
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Failed(message.clone());
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
                self.state.containers.discovering = false;
                self.state.containers.discovery = Some(ContainerDiscovery {
                    candidates: engines,
                    failures,
                });
                cx.notify();
            }
            ContainerAppEvent::TestFinished {
                request_id,
                info,
                error,
            } => {
                if let Some(form) = self.state.containers.form.as_mut()
                    && form.test_request == Some(request_id)
                {
                    form.test_request = None;
                    match (info, error) {
                        (Some(info), _) => {
                            form.test_ok = true;
                            form.test_status = Some(
                                rust_i18n::t!(
                                    "container.form.test_ok",
                                    platform = info.platform,
                                    version = info.version
                                )
                                .to_string(),
                            );
                        }
                        (None, Some(error)) => {
                            form.test_ok = false;
                            form.test_status = Some(localized_error(&error));
                        }
                        (None, None) => {
                            form.test_ok = false;
                            form.test_status =
                                Some(rust_i18n::t!("container.form.test_failed").to_string());
                        }
                    }
                }
            }
            ContainerAppEvent::Disconnected { tab_id } => {
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Disconnected;
                }
            }
            _ => {}
        }
    }
}
