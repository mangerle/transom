//! 容器工作区的选择、行操作与请求取消动作。

use super::*;

mod requests;

impl AppShell {
    pub(super) fn send_tracked(
        &mut self,
        tab_id: TabId,
        request_id: ContainerRequestId,
        command: ContainerCommand,
    ) {
        // 发送时就登记动作描述：结果事件只带 request_id，提示要靠它说清「做了什么」。
        let label = command_toast_label(&command);
        self.state.containers.track(request_id, tab_id, label);
        if !self.state.send_command(command) {
            self.state.containers.requests.remove(&request_id);
        }
    }

    /// 刷新当前分类的列表。
    pub(crate) fn refresh_container_section(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
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

    /// 切换顶部功能分类：清空选中项并拉取该分类列表。
    pub(crate) fn select_container_section(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_container_streams(tab_id);
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.selected = Some(id.clone());
            workspace.details = None;
            workspace.logs.clear();
            workspace.stats = None;
            workspace.stats_history.clear();
            workspace.stats_last_append = None;
            workspace.stats_error = None;
            workspace.stats_processes.clear();
            workspace.stats_processes_error = None;
            workspace.stats_processes_loading = false;
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.selected = Some(name);
            workspace.detail_tab = ContainerDetailTab::Overview;
        }
    }

    /// 让某一行成为选中项；已经选中时不重复拉详情，否则会把正在看的日志与
    /// 统计清掉。
    pub(crate) fn select_row(&mut self, tab_id: TabId, target: &RowTarget, cx: &mut Context<Self>) {
        let already_selected = self
            .state
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
        // 换条目回到结构化视图：上一个容器的原始 JSON 留在这一条上没有意义。
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.overview_raw = false;
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
            self.copy_row_text(tab_id, &target, action, window, cx);
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
            RowAction::Start
            | RowAction::Stop
            | RowAction::Restart
            | RowAction::Pause
            | RowAction::Unpause
            | RowAction::Kill => {
                let Some(name) = container else { return };
                let action = match action {
                    RowAction::Start => ContainerLifecycleAction::Start,
                    RowAction::Stop => ContainerLifecycleAction::Stop,
                    RowAction::Restart => ContainerLifecycleAction::Restart,
                    RowAction::Pause => ContainerLifecycleAction::Pause,
                    RowAction::Unpause => ContainerLifecycleAction::Unpause,
                    RowAction::Kill => ContainerLifecycleAction::Kill { signal: None },
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
                    (confirm, self.state.container_workspace_mut(tab_id))
                {
                    workspace.confirm = Some(confirm);
                    cx.notify();
                }
            }
            RowAction::Run => {
                let image = match &target {
                    RowTarget::Image(id) => self
                        .state
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
            RowAction::ExportConfig => {
                let RowTarget::Container { name, .. } = target else {
                    return;
                };
                self.open_container_export(tab_id, name, cx);
            }
            RowAction::Commit => {
                let RowTarget::Container { name, .. } = target else {
                    return;
                };
                self.open_container_commit(tab_id, name, window, cx);
            }
            RowAction::Tag => {
                let RowTarget::Image(id) = target else {
                    return;
                };
                self.open_image_tag(tab_id, id, window, cx);
            }
            RowAction::Push => {
                let RowTarget::Image(id) = target else {
                    return;
                };
                let reference = self
                    .state
                    .container_workspace(tab_id)
                    .and_then(|workspace| {
                        workspace
                            .images
                            .iter()
                            .find(|item| item.id == id)
                            .map(|item| item.display_name())
                    })
                    .unwrap_or_else(|| id.clone());
                let request_id = ContainerRequestId::new();
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.busy = true;
                }
                self.send_tracked(
                    tab_id,
                    request_id,
                    ContainerCommand::PushImage {
                        tab_id,
                        request_id,
                        image: reference,
                        tag: None,
                        auth: None,
                    },
                );
                cx.notify();
            }
            RowAction::EditComposeYaml => {
                let RowTarget::Compose(name) = target else {
                    return;
                };
                self.edit_compose_yaml(tab_id, &name, window, cx);
            }
            RowAction::CopyId | RowAction::CopyName => {}
        }
    }

    pub(crate) fn set_container_detail_tab(
        &mut self,
        tab_id: TabId,
        tab: ContainerDetailTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let previous = workspace.detail_tab;
        workspace.detail_tab = tab;
        let logs = workspace.logs_request;
        let stats = workspace.stats_request;
        let terminal = workspace
            .terminal_request
            .as_ref()
            .map(|(request_id, ..)| *request_id);

        if previous == ContainerDetailTab::Logs
            && tab != ContainerDetailTab::Logs
            && let Some(request_id) = logs
        {
            self.cancel_container_request(tab_id, request_id);
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.logs_request = None;
                workspace.logs_follow = false;
            }
        }
        if previous == ContainerDetailTab::Stats
            && tab != ContainerDetailTab::Stats
            && let Some(request_id) = stats
        {
            self.cancel_container_request(tab_id, request_id);
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.stats_request = None;
            }
        }
        if previous == ContainerDetailTab::Stats && tab != ContainerDetailTab::Stats {
            let processes = self
                .state
                .container_workspace_mut(tab_id)
                .and_then(|workspace| workspace.stats_processes_request.take());
            if let Some(request_id) = processes {
                self.cancel_container_request(tab_id, request_id);
                if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                    workspace.stats_processes_loading = false;
                }
            }
        }
        // 还没建立完的终端：用户已经切走就撤销，避免后台建好后又把页签抢回去。
        if previous == ContainerDetailTab::Terminal
            && tab != ContainerDetailTab::Terminal
            && let Some(request_id) = terminal
        {
            self.cancel_container_request(tab_id, request_id);
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.terminal_request = None;
            }
        }

        match tab {
            ContainerDetailTab::Logs => self.ensure_container_logs(tab_id),
            ContainerDetailTab::Stats => {
                self.ensure_container_stats(tab_id, cx);
                self.ensure_container_processes(tab_id);
            }
            ContainerDetailTab::Files => self.ensure_container_files(tab_id),
            ContainerDetailTab::Terminal => self.ensure_container_terminal(tab_id, None, None, cx),
            ContainerDetailTab::Overview => {}
        }
        cx.notify();
    }

    /// 从编排/卷/网络详情跳到某个容器：必要时先切分类并拉列表。
    pub(crate) fn reveal_container(&mut self, tab_id: TabId, name: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
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
}
