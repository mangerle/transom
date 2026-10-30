//! 容器引擎管理工作区：状态、命令与事件路由。

mod detail;
mod form;
mod list;
mod types;
mod view;

pub(crate) use types::{
    ContainerBatchAction, ContainerConfirm, ContainerDetailTab, ContainerDiscovery,
    ContainerFilesState, ContainerSection, ContainerTerminalState, ContainerUiState,
    ContainerWorkspaceState, ContainerWorkspaceStatus, RowAction, RowTarget,
};

use crate::app::{AppShell, TabContent, TabStatus, TerminalTab};
use crate::container::types::{arm_or_fire, row_key};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{
    Connection, ContainerAppEvent, ContainerCommand, ContainerConnection, ContainerEngineProfile,
    ContainerLifecycleAction, ContainerRequestId, Session, TabId, TerminalSize,
};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

impl AppShell {
    /// 容器事件分发：先按请求 ID 找到工作区，再更新对应字段。
    pub(crate) fn dispatch_container_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::Connected { tab_id, info } => {
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Connected;
                    workspace.info = Some(*info);
                    workspace.notice = None;
                }
                if let Some(tab) = self.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Connected;
                }
                self.refresh_container_section(tab_id);
            }
            ContainerAppEvent::ConnectFailed { tab_id, error } => {
                let message = localized_error(&error);
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Failed(message.clone());
                    workspace.notice = Some(message);
                }
                if let Some(tab) = self.workspace.tab_mut(tab_id) {
                    tab.status = TabStatus::Failed(error);
                }
            }
            ContainerAppEvent::EnginesDiscovered {
                engines, failures, ..
            } => {
                self.containers.discovering = false;
                self.containers.discovery = Some(ContainerDiscovery {
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
                if let Some(form) = self.containers.form.as_mut()
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
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.status = ContainerWorkspaceStatus::Disconnected;
                }
            }
            ContainerAppEvent::ContainersListed {
                request_id,
                containers,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                let mut inspect_first = None;
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
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
                }
                if let Some(id) = inspect_first {
                    self.select_container_item(tab_id, id);
                }
            }
            ContainerAppEvent::ContainerInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.id.clone());
                    workspace.details = Some(*details);
                }
            }
            ContainerAppEvent::ImagesListed { request_id, images } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.images = images;
                    workspace.busy = false;
                    let valid = workspace
                        .selected
                        .as_deref()
                        .is_some_and(|id| workspace.images.iter().any(|item| item.id == id));
                    if !valid {
                        workspace.selected = workspace.images.first().map(|item| item.id.clone());
                        workspace.image_details = None;
                    }
                }
            }
            ContainerAppEvent::ImageInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.id.clone());
                    workspace.image_details = Some(*details);
                }
            }
            ContainerAppEvent::VolumesListed {
                request_id,
                volumes,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.volumes = volumes;
                    workspace.busy = false;
                    let valid = workspace
                        .selected
                        .as_deref()
                        .is_some_and(|name| workspace.volumes.iter().any(|item| item.name == name));
                    if !valid {
                        workspace.selected =
                            workspace.volumes.first().map(|item| item.name.clone());
                        workspace.volume_details = None;
                    }
                }
            }
            ContainerAppEvent::VolumeInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.summary.name.clone());
                    workspace.volume_details = Some(*details);
                }
            }
            ContainerAppEvent::NetworksListed {
                request_id,
                networks,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.networks = networks;
                    workspace.busy = false;
                    let valid = workspace
                        .selected
                        .as_deref()
                        .is_some_and(|id| workspace.networks.iter().any(|item| item.id == id));
                    if !valid {
                        workspace.selected = workspace.networks.first().map(|item| item.id.clone());
                        workspace.network_details = None;
                    }
                }
            }
            ContainerAppEvent::NetworkInspected {
                request_id,
                details,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.selected = Some(details.summary.id.clone());
                    workspace.network_details = Some(*details);
                }
            }
            ContainerAppEvent::ComposeProjectsListed {
                request_id,
                projects,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.compose = projects;
                    workspace.busy = false;
                }
            }
            ContainerAppEvent::ExecReady {
                tab_id,
                request_id,
                io,
                exit,
            } => {
                self.containers.requests.remove(&request_id);
                drop(exit);
                if self.workspace.tab(tab_id).is_none() {
                    return;
                }
                let (terminal, subscription) =
                    self.create_container_terminal(*io, tab_id, window, cx);
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.terminal = Some(ContainerTerminalState {
                        view: terminal,
                        _subscription: subscription,
                    });
                    workspace.detail_tab = ContainerDetailTab::Terminal;
                    workspace.notice = None;
                }
            }
            ContainerAppEvent::LogsChunk { request_id, bytes } => {
                let Some(tab_id) = self.containers.tab_of(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id)
                    && workspace.logs_request == Some(request_id)
                {
                    workspace.append_logs(&bytes);
                }
            }
            ContainerAppEvent::LogsClosed { request_id } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id)
                    && workspace.logs_request == Some(request_id)
                {
                    workspace.logs_request = None;
                    workspace.logs_follow = false;
                }
            }
            ContainerAppEvent::StatsUpdated {
                request_id,
                container: _,
                stats,
            } => {
                let Some(tab_id) = self.containers.tab_of(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id)
                    && workspace.stats_request == Some(request_id)
                {
                    workspace.stats = Some(stats);
                }
            }
            ContainerAppEvent::DirListed {
                request_id,
                path,
                entries,
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.files.path = path;
                    workspace.files.entries = entries;
                    workspace.files.loading = false;
                }
            }
            ContainerAppEvent::FilePreview {
                request_id,
                path,
                content,
                truncated,
                ..
            } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.files.preview = Some((path, content, truncated));
                }
            }
            ContainerAppEvent::TransferProgress {
                request_id,
                transferred,
                total,
            } => {
                let Some(tab_id) = self.containers.tab_of(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.notice = Some(match total {
                        Some(total) if total > 0 => format!(
                            "{} / {}",
                            crate::format::format_size(transferred),
                            crate::format::format_size(total)
                        ),
                        _ => crate::format::format_size(transferred),
                    });
                }
            }
            ContainerAppEvent::ActionFinished { request_id, error } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                let mut refresh = false;
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.busy = false;
                    workspace.confirm = None;
                    workspace.rename_container = None;
                    match error {
                        Some(error) => workspace.notice = Some(localized_error(&error)),
                        None => {
                            workspace.notice = None;
                            refresh = true;
                        }
                    }
                }
                if refresh {
                    self.refresh_container_section(tab_id);
                }
            }
            ContainerAppEvent::TransferFinished { request_id, error } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                let mut refresh_files = None;
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    match error {
                        Some(error) => workspace.notice = Some(localized_error(&error)),
                        None => {
                            workspace.notice = None;
                            refresh_files = Some(workspace.files.path.clone());
                        }
                    }
                }
                if let Some(path) = refresh_files
                    && !path.is_empty()
                {
                    self.load_container_dir(tab_id, path);
                }
            }
            ContainerAppEvent::Failed { request_id, error } => {
                let Some(tab_id) = self.containers.take_tab(request_id) else {
                    return;
                };
                if let Some(workspace) = self.container_workspace_mut(tab_id) {
                    workspace.busy = false;
                    workspace.files.loading = false;
                    if workspace.logs_request == Some(request_id) {
                        workspace.logs_request = None;
                        workspace.logs_follow = false;
                    }
                    if workspace.stats_request == Some(request_id) {
                        workspace.stats_request = None;
                    }
                    workspace.notice = Some(localized_error(&error));
                }
            }
        }
        cx.notify();
    }

    fn container_workspace_mut(&mut self, tab_id: TabId) -> Option<&mut ContainerWorkspaceState> {
        match &mut self.workspace.tab_mut(tab_id)?.content {
            TabContent::Containers(state) => Some(state),
            _ => None,
        }
    }

    /// 汇总每个引擎连接当前的工作区状态，供侧栏行首的状态点使用。
    pub(crate) fn container_profile_states(
        &self,
    ) -> std::collections::HashMap<transom_core::EngineProfileId, ContainerWorkspaceStatus> {
        let mut states = std::collections::HashMap::default();
        for tab in self.workspace.tabs.iter() {
            let Connection::ContainerEngine(profile) = &tab.session.connection else {
                continue;
            };
            let TabContent::Containers(workspace) = &tab.content else {
                continue;
            };
            states.insert(profile.id, workspace.status.clone());
        }
        states
    }

    pub(crate) fn container_workspace(&self, tab_id: TabId) -> Option<&ContainerWorkspaceState> {
        match &self.workspace.tab(tab_id)?.content {
            TabContent::Containers(state) => Some(state),
            _ => None,
        }
    }

    fn send_tracked(
        &mut self,
        tab_id: TabId,
        request_id: ContainerRequestId,
        command: ContainerCommand,
    ) {
        self.containers.track(request_id, tab_id);
        if !self.send_command(command) {
            self.containers.requests.remove(&request_id);
        }
    }

    /// 打开（或聚焦）一个引擎连接的工作区标签。
    pub(crate) fn open_engine_workspace(
        &mut self,
        profile: ContainerEngineProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) =
            self.workspace
                .tabs
                .iter()
                .find_map(|tab| match &tab.session.connection {
                    Connection::ContainerEngine(existing) if existing.id == profile.id => {
                        Some(tab.id)
                    }
                    _ => None,
                })
        {
            self.activate_tab(existing, window, cx);
            return;
        }
        let tab_id = TabId::new();
        let session = Session::new(
            profile.display_name(),
            Connection::ContainerEngine(profile.clone()),
        );
        self.create_group(tab_id, session.id);
        let mut tab = TerminalTab::pending(tab_id, session);
        tab.status = TabStatus::Connecting;
        tab.title = profile.display_name();
        tab.content = TabContent::Containers(Box::new(ContainerWorkspaceState::new(
            profile.clone(),
            window,
            cx,
        )));
        self.workspace.tabs.push(tab);
        self.workspace.active_tab = Some(tab_id);
        self.present_active_group();
        self.send_command(ContainerCommand::Connect { tab_id, profile });
        cx.notify();
    }

    /// 刷新当前分类的列表。
    pub(crate) fn refresh_container_section(&mut self, tab_id: TabId) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let section = workspace.section;
        workspace.busy = true;
        let request_id = ContainerRequestId::new();
        let command = match section {
            ContainerSection::Containers | ContainerSection::Compose => {
                ContainerCommand::ListContainers { tab_id, request_id }
            }
            ContainerSection::Images => ContainerCommand::ListImages { tab_id, request_id },
            ContainerSection::Volumes => ContainerCommand::ListVolumes { tab_id, request_id },
            ContainerSection::Networks => ContainerCommand::ListNetworks { tab_id, request_id },
        };
        self.send_tracked(tab_id, request_id, command);
        // 卷分类要同时拉容器列表：「在用 / 空闲」与「仅未使用卷」都靠容器的挂载
        // 明细算，缺了这份数据会把在用卷说成空闲，用户据此删除就是事故。
        if section == ContainerSection::Volumes {
            let usage_request = ContainerRequestId::new();
            self.send_tracked(
                tab_id,
                usage_request,
                ContainerCommand::ListContainers {
                    tab_id,
                    request_id: usage_request,
                },
            );
        }
    }

    /// 切换左侧分类：清空选中项并拉取该分类列表。
    pub(crate) fn select_container_section(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_container_streams(tab_id);
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.apply_section(section);
            workspace
                .create_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.refresh_container_section(tab_id);
        cx.notify();
    }

    /// 选中一个容器：停止旧订阅并拉取详情。
    pub(crate) fn select_container_item(&mut self, tab_id: TabId, id: String) {
        self.stop_container_streams(tab_id);
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.selected = Some(id.clone());
            workspace.details = None;
            workspace.logs.clear();
            workspace.stats = None;
            workspace.files = ContainerFilesState::default();
            workspace.detail_tab = ContainerDetailTab::Overview;
            workspace.confirm = None;
            workspace.notice = None;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::InspectContainer {
                tab_id,
                request_id,
                container: id,
            },
        );
    }

    pub(crate) fn select_image_item(&mut self, tab_id: TabId, id: String) {
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.selected = Some(id.clone());
            workspace.image_details = None;
            workspace.detail_tab = ContainerDetailTab::Overview;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::InspectImage {
                tab_id,
                request_id,
                image: id,
            },
        );
    }

    pub(crate) fn select_volume_item(&mut self, tab_id: TabId, name: String) {
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.selected = Some(name.clone());
            workspace.volume_details = None;
            workspace.detail_tab = ContainerDetailTab::Overview;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::InspectVolume {
                tab_id,
                request_id,
                name,
            },
        );
    }

    pub(crate) fn select_network_item(&mut self, tab_id: TabId, id: String) {
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.selected = Some(id.clone());
            workspace.network_details = None;
            workspace.detail_tab = ContainerDetailTab::Overview;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::InspectNetwork {
                tab_id,
                request_id,
                network: id,
            },
        );
    }

    pub(crate) fn select_compose_item(&mut self, tab_id: TabId, name: String) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.selected = Some(name);
            workspace.detail_tab = ContainerDetailTab::Overview;
        }
    }

    /// 让某一行成为选中项；已经选中时不重复拉详情，否则会把正在看的日志与
    /// 统计清掉。
    pub(crate) fn select_row(&mut self, tab_id: TabId, target: &RowTarget, cx: &mut Context<Self>) {
        let already_selected = self
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.selected.clone())
            .is_some_and(|selected| selected == row_key(target));
        if already_selected {
            return;
        }
        match target {
            RowTarget::Container { id, .. } => self.select_container_item(tab_id, id.clone()),
            RowTarget::Image(id) => self.select_image_item(tab_id, id.clone()),
            RowTarget::Volume(name) => self.select_volume_item(tab_id, name.clone()),
            RowTarget::Network(id) => self.select_network_item(tab_id, id.clone()),
            RowTarget::Compose(name) => self.select_compose_item(tab_id, name.clone()),
        }
        cx.notify();
    }

    /// 列表行动作分发：行内快捷按钮与右键菜单共用这一套语义，避免出现两处行为。
    ///
    /// 复制类动作不改选中项：用户常常要复制一个不是当前选中项的 ID，改选中会
    /// 顺手清掉右侧正在看的内容。
    pub(crate) fn run_row_action(
        &mut self,
        tab_id: TabId,
        target: RowTarget,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(action, RowAction::CopyId | RowAction::CopyName) {
            self.copy_row_text(tab_id, &target, action, cx);
            return;
        }
        self.select_row(tab_id, &target, cx);
        let container = match &target {
            RowTarget::Container { name, .. } => Some(name.clone()),
            _ => None,
        };
        match action {
            RowAction::Overview => {
                self.set_container_detail_tab(tab_id, ContainerDetailTab::Overview, window, cx);
            }
            RowAction::Logs => {
                self.set_container_detail_tab(tab_id, ContainerDetailTab::Logs, window, cx);
            }
            RowAction::Terminal => {
                self.set_container_detail_tab(tab_id, ContainerDetailTab::Terminal, window, cx);
            }
            RowAction::Stats => {
                self.set_container_detail_tab(tab_id, ContainerDetailTab::Stats, window, cx);
            }
            RowAction::Files => {
                self.set_container_detail_tab(tab_id, ContainerDetailTab::Files, window, cx);
            }
            RowAction::Start | RowAction::Stop | RowAction::Restart => {
                let Some(name) = container else { return };
                let action = match action {
                    RowAction::Start => ContainerLifecycleAction::Start,
                    RowAction::Stop => ContainerLifecycleAction::Stop,
                    _ => ContainerLifecycleAction::Restart,
                };
                self.container_lifecycle(tab_id, name, action, cx);
            }
            RowAction::Remove => {
                let confirm = match target {
                    RowTarget::Container { name, .. } => Some(ContainerConfirm::Container(name)),
                    RowTarget::Image(id) => Some(ContainerConfirm::Image(id)),
                    RowTarget::Volume(name) => Some(ContainerConfirm::Volume(name)),
                    RowTarget::Network(id) => Some(ContainerConfirm::Network(id)),
                    RowTarget::Compose(_) => None,
                };
                if let (Some(confirm), Some(workspace)) =
                    (confirm, self.container_workspace_mut(tab_id))
                {
                    workspace.confirm = Some(confirm);
                    cx.notify();
                }
            }
            RowAction::Run => {
                let image = match &target {
                    RowTarget::Image(id) => self
                        .container_workspace(tab_id)
                        .and_then(|workspace| {
                            workspace
                                .images
                                .iter()
                                .find(|image| &image.id == id)
                                .map(|image| image.display_name())
                        })
                        .unwrap_or_else(|| transom_core::short_id(id)),
                    _ => return,
                };
                self.open_container_run_form(tab_id, Some(image), window, cx);
            }
            RowAction::Rename => {
                let RowTarget::Container { id, name } = target else {
                    return;
                };
                self.open_container_rename(tab_id, id, name, window, cx);
            }
            RowAction::CopyId | RowAction::CopyName => {}
        }
    }

    /// 复制行的标识到剪贴板，并在头部给出可检索的回执。
    fn copy_row_text(
        &mut self,
        tab_id: TabId,
        target: &RowTarget,
        action: RowAction,
        cx: &mut Context<Self>,
    ) {
        let text = match (target, action) {
            (RowTarget::Container { id, name }, action) => {
                if action == RowAction::CopyId {
                    id.clone()
                } else {
                    name.clone()
                }
            }
            (RowTarget::Volume(name), _) | (RowTarget::Compose(name), _) => name.clone(),
            (RowTarget::Network(id), RowAction::CopyId) => id.clone(),
            (RowTarget::Network(id), RowAction::CopyName) => self
                .container_workspace(tab_id)
                .and_then(|workspace| {
                    workspace
                        .networks
                        .iter()
                        .find(|network| &network.id == id)
                        .map(|network| network.name.clone())
                })
                .unwrap_or_else(|| id.clone()),
            (RowTarget::Image(id), action) => {
                let workspace = self.container_workspace(tab_id);
                match action {
                    RowAction::CopyId => id.clone(),
                    _ => workspace
                        .and_then(|workspace| {
                            workspace
                                .images
                                .iter()
                                .find(|image| &image.id == id)
                                .map(|image| image.display_name())
                        })
                        .unwrap_or_else(|| transom_core::short_id(id)),
                }
            }
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.notice =
                Some(rust_i18n::t!("container.row.copied", value = text).to_string());
        }
        cx.notify();
    }

    /// 停止当前选中项的日志、统计与终端会话。
    fn stop_container_streams(&mut self, tab_id: TabId) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let logs = workspace.logs_request.take();
        let stats = workspace.stats_request.take();
        workspace.terminal = None;
        for request_id in [logs, stats].into_iter().flatten() {
            self.containers.requests.remove(&request_id);
            self.send_command(ContainerCommand::Cancel { tab_id, request_id });
        }
    }

    /// 立即取消一个流式请求（切换页签时使用）。
    pub(crate) fn cancel_container_request(
        &mut self,
        tab_id: TabId,
        request_id: ContainerRequestId,
    ) {
        self.containers.requests.remove(&request_id);
        self.send_command(ContainerCommand::Cancel { tab_id, request_id });
    }

    pub(crate) fn set_container_detail_tab(
        &mut self,
        tab_id: TabId,
        tab: ContainerDetailTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let previous = workspace.detail_tab;
        workspace.detail_tab = tab;
        let logs = workspace.logs_request;
        let stats = workspace.stats_request;

        if previous == ContainerDetailTab::Logs
            && tab != ContainerDetailTab::Logs
            && let Some(request_id) = logs
        {
            self.cancel_container_request(tab_id, request_id);
            if let Some(workspace) = self.container_workspace_mut(tab_id) {
                workspace.logs_request = None;
                workspace.logs_follow = false;
            }
        }
        if previous == ContainerDetailTab::Stats
            && tab != ContainerDetailTab::Stats
            && let Some(request_id) = stats
        {
            self.cancel_container_request(tab_id, request_id);
            if let Some(workspace) = self.container_workspace_mut(tab_id) {
                workspace.stats_request = None;
            }
        }

        match tab {
            ContainerDetailTab::Logs => self.ensure_container_logs(tab_id),
            ContainerDetailTab::Stats => self.ensure_container_stats(tab_id),
            ContainerDetailTab::Files => self.ensure_container_files(tab_id),
            ContainerDetailTab::Terminal => self.ensure_container_terminal(tab_id, window, cx),
            ContainerDetailTab::Overview => {}
        }
        cx.notify();
    }

    pub(crate) fn ensure_container_logs(&mut self, tab_id: TabId) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.logs_request.is_some() {
            return;
        }
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let follow = workspace.logs_follow;
        let tail = workspace.logs_tail;
        let timestamps = workspace.logs_timestamps;
        workspace.logs.clear();
        let request_id = ContainerRequestId::new();
        workspace.logs_request = Some(request_id);
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Logs {
                tab_id,
                request_id,
                container,
                tail,
                follow,
                timestamps,
            },
        );
    }

    /// 重开日志流：跟随、尾部行数与时间戳都要改才能生效，先取消旧流再拉新流。
    fn restart_container_logs(&mut self, tab_id: TabId) {
        if let Some(request_id) = self
            .container_workspace_mut(tab_id)
            .and_then(|workspace| workspace.logs_request.take())
        {
            self.cancel_container_request(tab_id, request_id);
        }
        self.ensure_container_logs(tab_id);
    }

    /// 切换日志尾部行数档位（0 表示全量）。
    pub(crate) fn set_container_logs_tail(&mut self, tab_id: TabId, tail: usize) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            if workspace.logs_tail == tail {
                return;
            }
            workspace.logs_tail = tail;
        }
        self.restart_container_logs(tab_id);
    }

    /// 开关日志时间戳：由引擎附带时间戳，界面不做二次格式化。
    pub(crate) fn toggle_container_logs_timestamps(&mut self, tab_id: TabId) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.logs_timestamps = !workspace.logs_timestamps;
        }
        self.restart_container_logs(tab_id);
    }

    /// 把当前日志缓冲整体复制到剪贴板，便于贴到工单或聊天里。
    pub(crate) fn copy_container_logs(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(text) = self
            .container_workspace(tab_id)
            .map(|workspace| workspace.logs.clone())
        else {
            return;
        };
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.notify();
    }

    pub(crate) fn toggle_container_logs_follow(&mut self, tab_id: TabId, follow: bool) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.logs_follow = follow;
        }
        self.restart_container_logs(tab_id);
    }

    pub(crate) fn ensure_container_stats(&mut self, tab_id: TabId) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.stats_request.is_some() {
            return;
        }
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.stats_request = Some(request_id);
        workspace.stats = None;
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Stats {
                tab_id,
                request_id,
                container,
                enabled: true,
            },
        );
    }

    /// Esc 关闭容器侧的浮层：先关表单，再退全屏。
    ///
    /// 为什么挂在浮层自己的按键回调上而不是全局键位：终端里 Esc 是 vim 等程序的
    /// 正常输入，全局绑一个「关浮层」的键会在没有浮层时也吃掉这一次按键。浮层
    /// 存在时它的元素在焦点路径上，按键自然只到这里；全屏视图里正处于终端页签时
    /// 直接放行，Esc 仍然只交给容器内的程序。
    pub(crate) fn on_container_overlay_key(
        &mut self,
        event: &KeyDownEvent,
        fullscreen_tab: Option<TabId>,
        cx: &mut Context<Self>,
    ) {
        if !event.keystroke.key.eq_ignore_ascii_case("escape") {
            return;
        }
        if self.containers.run_form.is_some() {
            self.dismiss_container_run(cx);
            return;
        }
        if self.containers.form.is_some() {
            self.containers.form = None;
            cx.notify();
            return;
        }
        let Some(tab) = fullscreen_tab.filter(|tab| self.containers.fullscreen == Some(*tab))
        else {
            return;
        };
        // 终端页签里 Esc 是 vim 等程序的正常输入，这时不能把整个工作区收掉；
        // 其余页签（列表/日志/统计/文件）Esc 的语义就是「关掉眼前的全屏」。
        let in_terminal = self
            .container_workspace(tab)
            .is_some_and(|workspace| workspace.detail_tab == ContainerDetailTab::Terminal);
        if in_terminal {
            return;
        }
        self.containers.fullscreen = None;
        cx.notify();
    }

    /// 打开容器改名条。
    pub(crate) fn open_container_rename(
        &mut self,
        tab_id: TabId,
        id: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.rename_container = Some((id, input));
        }
        cx.notify();
    }

    /// 取消改名。
    pub(crate) fn cancel_container_rename(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.rename_container = None;
        }
        cx.notify();
    }

    /// 提交改名：名称留空或与原名相同都不下发，避免无意义的请求与 409。
    pub(crate) fn submit_container_rename(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((id, input)) = self
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.rename_container.clone())
        else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        let same = self
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.containers.iter().find(|item| item.id == id))
            .is_some_and(|item| item.name == name);
        if name.is_empty() || same {
            if let Some(workspace) = self.container_workspace_mut(tab_id) {
                workspace.notice =
                    Some(rust_i18n::t!("container.rename.invalid", name = name).to_string());
            }
            cx.notify();
            return;
        }
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.busy = true;
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::RenameContainer {
                tab_id,
                request_id,
                container: id,
                name,
            },
        );
        cx.notify();
    }

    /// 编排项目命令：`up` 直接下发，`down` 会删容器与网络，先过确认条。
    pub(crate) fn run_compose(
        &mut self,
        tab_id: TabId,
        name: String,
        action: transom_core::ComposeAction,
        cx: &mut Context<Self>,
    ) {
        if action.is_dangerous() {
            if let Some(workspace) = self.container_workspace_mut(tab_id) {
                workspace.confirm = Some(ContainerConfirm::Compose(name));
            }
            cx.notify();
            return;
        }
        let Some(project) = self.container_workspace(tab_id).and_then(|workspace| {
            workspace
                .compose
                .iter()
                .find(|item| item.name == name)
                .cloned()
        }) else {
            return;
        };
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.notice = Some(
                rust_i18n::t!("container.compose.running", action = action.label()).to_string(),
            );
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::RunCompose {
                tab_id,
                request_id,
                project: Box::new(project),
                action,
            },
        );
        cx.notify();
    }

    /// 切换容器勾选状态（多选）。
    pub(crate) fn toggle_container_check(
        &mut self,
        tab_id: TabId,
        id: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
            workspace.toggle_checked(&id);
        }
        cx.notify();
    }

    /// 批量下发启停/删除；删除在同一批勾选下必须连点两次，避免误删一片。
    pub(crate) fn run_container_batch(
        &mut self,
        tab_id: TabId,
        action: ContainerBatchAction,
        cx: &mut Context<Self>,
    ) {
        let names = {
            let Some(workspace) = self.container_workspace_mut(tab_id) else {
                return;
            };
            let names = workspace.checked_names();
            if action.is_dangerous() {
                let (armed, fire) = arm_or_fire(workspace.batch_armed, names.len());
                workspace.batch_armed = armed;
                if !fire {
                    cx.notify();
                    return;
                }
            }
            workspace.checked.clear();
            workspace.batch_armed = false;
            names
        };
        // 每个容器一条独立命令：引擎侧本来就是按容器逐个处理，失败也只影响自己，
        // 控制层按 request_id 回报，界面不需要额外的批量事务语义。
        for name in names {
            self.container_lifecycle(tab_id, name, action.lifecycle(), cx);
        }
        cx.notify();
    }

    /// 从编排/卷/网络详情跳到某个容器：必要时先切分类并拉列表。
    pub(crate) fn reveal_container(&mut self, tab_id: TabId, name: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let already_visible = workspace.section == ContainerSection::Containers
            && workspace.containers.iter().any(|item| item.name == name);
        if already_visible {
            if let Some(id) = workspace
                .containers
                .iter()
                .find(|item| item.name == name)
                .map(|item| item.id.clone())
            {
                self.select_container_item(tab_id, id);
                cx.notify();
            }
            return;
        }
        let need_list =
            workspace.section != ContainerSection::Containers || workspace.containers.is_empty();
        workspace.apply_section(ContainerSection::Containers);
        workspace.pending_select = Some(name);
        if need_list {
            self.stop_container_streams(tab_id);
            self.refresh_container_section(tab_id);
        }
        cx.notify();
    }

    /// 开关统计自动刷新：关掉时通知引擎停止轮询，界面保留最后一帧。
    pub(crate) fn toggle_container_stats(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let enabled = !workspace.stats_enabled;
        workspace.stats_enabled = enabled;
        if enabled {
            if workspace.stats_request.is_none() {
                self.ensure_container_stats(tab_id);
            }
        } else if let Some(request_id) = workspace.stats_request.take() {
            let cancel = request_id;
            self.send_tracked(
                tab_id,
                cancel,
                ContainerCommand::Stats {
                    tab_id,
                    request_id: cancel,
                    container: container.clone(),
                    enabled: false,
                },
            );
        }
        cx.notify();
    }

    pub(crate) fn ensure_container_files(&mut self, tab_id: TabId) {
        let path = self
            .container_workspace_mut(tab_id)
            .map(|workspace| workspace.files.path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/".to_string());
        self.load_container_dir(tab_id, path);
    }

    pub(crate) fn load_container_dir(&mut self, tab_id: TabId, path: String) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        workspace.files.loading = true;
        workspace.files.preview = None;
        let request_id = ContainerRequestId::new();
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ListDir {
                tab_id,
                request_id,
                container,
                path,
            },
        );
    }

    pub(crate) fn ensure_container_terminal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.terminal.is_some() {
            return;
        }
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ExecTerminal {
                tab_id,
                request_id,
                options: ContainerConnection::new(container),
                size: TerminalSize::new(100, 30),
            },
        );
        let _ = (window, cx);
    }

    fn create_container_terminal(
        &mut self,
        io: transom_core::TerminalIo,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<TerminalView>, Subscription) {
        let mut profile = self.settings.terminal_profile.clone();
        profile.background_transparency = self.settings.window.window_transparency;
        let interaction = self.settings.terminal_interaction;
        let reserved = self.shortcut_recording.filter.clone();
        let key_filter: KeyFilter = Arc::new(move |keystroke| {
            reserved
                .read()
                .iter()
                .any(|binding| keystroke.should_match(binding))
        });
        let terminal = cx.new(|cx| {
            TerminalView::new(
                window,
                cx,
                transom_terminal::TerminalViewParams {
                    io,
                    profile,
                    interaction,
                    key_filter: Some(key_filter),
                    audit_path: None,
                    shell_integration_enabled: false,
                },
            )
        });
        let provider = self.history_provider.clone();
        terminal.update(cx, |view, _cx| view.attach_history(provider));
        let focus_handle = terminal.read(cx).focus_handle().clone();
        window.focus(&focus_handle, cx);
        let subscription = cx.subscribe_in(
            &terminal,
            window,
            move |this, _, event: &TerminalEvent, _window, cx| {
                if matches!(event, TerminalEvent::Exit) {
                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
                        workspace.terminal = None;
                        workspace.notice =
                            Some(rust_i18n::t!("container.terminal.closed").to_string());
                    }
                    cx.notify();
                }
            },
        );
        (terminal, subscription)
    }

    /// 保存引擎连接配置（新建或更新）。
    /// 发起一次本机引擎发现：用户不必手填 socket/管道路径。
    pub(crate) fn discover_engines(&mut self, cx: &mut Context<Self>) {
        if self.containers.discovering {
            return;
        }
        self.containers.discovering = true;
        self.containers.discovery = None;
        self.send_command(ContainerCommand::Discover {
            request_id: ContainerRequestId::new(),
        });
        cx.notify();
    }

    /// 收起发现结果面板。
    pub(crate) fn dismiss_container_discovery(&mut self, cx: &mut Context<Self>) {
        self.containers.discovery = None;
        cx.notify();
    }

    /// 添加一个发现的引擎：同种类同端点已保存时不再重复入库，直接打开工作区。
    pub(crate) fn add_discovered_engine(
        &mut self,
        profile: ContainerEngineProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let duplicated = self.catalog.engines.iter().any(|saved| {
            saved.engine.kind == profile.engine.kind
                && saved.engine.endpoint.describe() == profile.engine.endpoint.describe()
        });
        if !duplicated {
            self.catalog.engines.push(profile.clone());
            self.persist_catalog();
        }
        if let Some(discovery) = self.containers.discovery.as_mut() {
            discovery.candidates.retain(|candidate| {
                candidate.engine.endpoint.describe() != profile.engine.endpoint.describe()
            });
        }
        self.open_engine_workspace(profile, window, cx);
    }

    pub(crate) fn save_container_profile(
        &mut self,
        profile: ContainerEngineProfile,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) = self
            .catalog
            .engines
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            *existing = profile;
        } else {
            self.catalog.engines.push(profile);
        }
        self.persist_catalog();
        cx.notify();
    }

    /// 删除引擎连接配置并关闭其工作区标签。
    pub(crate) fn delete_container_profile(
        &mut self,
        id: transom_core::EngineProfileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalog.engines.retain(|profile| profile.id != id);
        self.persist_catalog();
        let tabs: Vec<TabId> = self
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| match &tab.session.connection {
                Connection::ContainerEngine(profile) if profile.id == id => Some(tab.id),
                _ => None,
            })
            .collect();
        for tab_id in tabs {
            self.close_tab(tab_id, window, cx);
        }
        cx.notify();
    }

    /// 取消当前分类里的删除确认。
    pub(crate) fn cancel_container_confirm(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
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
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
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
                let Some(project) = self.container_workspace(tab_id).and_then(|workspace| {
                    workspace
                        .compose
                        .iter()
                        .find(|item| item.name == project)
                        .cloned()
                }) else {
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
                let Some(workspace) = self.container_workspace_mut(tab_id) else {
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
        if let Some(workspace) = self.container_workspace_mut(tab_id) {
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
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let section = workspace.section;
        let value = workspace.create_input.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }
        let request_id = ContainerRequestId::new();
        let command = match section {
            ContainerSection::Images => ContainerCommand::PullImage {
                tab_id,
                request_id,
                reference: value,
            },
            ContainerSection::Volumes => ContainerCommand::CreateVolume {
                tab_id,
                request_id,
                name: value,
                driver: None,
            },
            ContainerSection::Networks => ContainerCommand::CreateNetwork {
                tab_id,
                request_id,
                name: value,
                driver: None,
            },
            _ => return,
        };
        workspace.busy = true;
        workspace
            .create_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }

    /// 预览容器内文本文件。
    pub(crate) fn preview_container_file(&mut self, tab_id: TabId, path: String) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ReadFile {
                tab_id,
                request_id,
                container,
                path,
                max_bytes: 1024 * 1024,
            },
        );
    }

    /// 下载容器内文件。
    pub(crate) fn download_container_file(&mut self, tab_id: TabId, path: String) {
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(path.rsplit('/').next().unwrap_or("download"))
            .save_file()
        else {
            return;
        };
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Download {
                tab_id,
                request_id,
                container,
                path,
                local_path,
            },
        );
    }

    /// 上传本地文件到当前目录。
    pub(crate) fn upload_container_file(&mut self, tab_id: TabId) {
        let Some(local_path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return;
        };
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let directory = if workspace.files.path.is_empty() {
            "/".to_string()
        } else {
            workspace.files.path.clone()
        };
        let target = if directory == "/" {
            format!("/{file_name}")
        } else {
            format!("{directory}/{file_name}")
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Upload {
                tab_id,
                request_id,
                container,
                path: target,
                local_path,
            },
        );
    }

    /// 重命名容器内路径。
    /// 展开或收起「新建目录」条；展开时才创建输入框，避免常驻实体。
    pub(crate) fn toggle_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.new_dir_open = !workspace.files.new_dir_open;
        if workspace.files.new_dir_open && workspace.files.new_dir_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("container.files.new_dir_placeholder").to_string())
            });
            workspace.files.new_dir_input = Some(input);
        }
        cx.notify();
    }

    /// 在容器当前目录里新建子目录：上传与整理文件都需要先有目录。
    pub(crate) fn submit_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let Some(input) = workspace.files.new_dir_input.clone() else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() || name.contains('/') || name.contains("..") {
            if let Some(workspace) = self.container_workspace_mut(tab_id) {
                workspace.notice =
                    Some(rust_i18n::t!("container.files.new_dir_invalid").to_string());
            }
            cx.notify();
            return;
        }
        let parent = if workspace.files.path.is_empty() {
            "/".to_string()
        } else {
            workspace.files.path.clone()
        };
        let path = if parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}/{name}")
        };
        let request_id = ContainerRequestId::new();
        workspace.files.new_dir_open = false;
        workspace.busy = true;
        workspace
            .files
            .new_dir_input
            .as_ref()
            .iter()
            .for_each(|input| input.update(cx, |input, cx| input.set_value("", window, cx)));
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::CreateDir {
                tab_id,
                request_id,
                container,
                path,
            },
        );
        cx.notify();
    }

    pub(crate) fn rename_container_path(&mut self, tab_id: TabId, from: String, to: String) {
        let Some(workspace) = self.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.files.rename_from = None;
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Rename {
                tab_id,
                request_id,
                container,
                from,
                to,
            },
        );
    }
}
