//! 容器工作区的选择、行操作与请求取消动作。

use super::*;

mod changes;
mod image_archive;
mod refresh;
mod requests;
mod volume_archive;

impl ContainerWorkspace {
    /// 对单个已连接工作区发起一次轻量清理；由应用级定时器调用。
    pub(crate) fn request_auto_prune(&mut self) {
        let request_id = ContainerRequestId::new();
        log::info!(
            "容器自动清理触发，标签: {}，请求: {request_id}",
            self.tab_id
        );
        self.send_tracked(
            request_id,
            ContainerCommand::SystemPrune {
                tab_id: self.tab_id,
                request_id,
                all: false,
                volumes: false,
            },
        );
    }

    /// 选中一个容器：停止旧订阅并拉取详情。
    pub(crate) fn select_container_item(&mut self, id: String) {
        self.stop_container_streams();
        let request_id = ContainerRequestId::new();
        self.state.selected = Some(id.clone());
        self.state.details = None;
        self.state.logs.clear();
        self.state.stats = None;
        self.state.stats_history.clear();
        self.state.stats_last_append = None;
        self.state.stats_error = None;
        self.state.stats_processes.clear();
        self.state.stats_processes_error = None;
        self.state.stats_processes_loading = false;
        self.state.files = ContainerFilesState::default();
        self.state.detail_tab = ContainerDetailTab::Overview;
        self.state.confirm = None;
        self.state.notice = None;
        self.send_tracked(
            request_id,
            ContainerCommand::InspectContainer {
                tab_id: self.tab_id,
                request_id,
                container: id,
            },
        );
    }

    pub(crate) fn select_image_item(&mut self, id: String) {
        let request_id = ContainerRequestId::new();
        self.state.selected = Some(id.clone());
        self.state.image_details = None;
        self.state.detail_tab = ContainerDetailTab::Overview;
        self.send_tracked(
            request_id,
            ContainerCommand::InspectImage {
                tab_id: self.tab_id,
                request_id,
                image: id,
            },
        );
    }

    pub(crate) fn select_volume_item(&mut self, name: String) {
        let request_id = ContainerRequestId::new();
        self.state.selected = Some(name.clone());
        self.state.volume_details = None;
        self.state.detail_tab = ContainerDetailTab::Overview;
        self.send_tracked(
            request_id,
            ContainerCommand::InspectVolume {
                tab_id: self.tab_id,
                request_id,
                name,
            },
        );
    }

    pub(crate) fn select_network_item(&mut self, id: String) {
        let request_id = ContainerRequestId::new();
        self.state.selected = Some(id.clone());
        self.state.network_details = None;
        self.state.detail_tab = ContainerDetailTab::Overview;
        self.send_tracked(
            request_id,
            ContainerCommand::InspectNetwork {
                tab_id: self.tab_id,
                request_id,
                network: id,
            },
        );
    }

    pub(crate) fn select_compose_item(&mut self, name: String) {
        self.state.selected = Some(name);
        self.state.detail_tab = ContainerDetailTab::Overview;
    }

    /// 让某一行成为选中项；已经选中时不重复拉详情，否则会把正在看的日志与
    /// 统计清掉。
    pub(crate) fn select_row(&mut self, target: &RowTarget, cx: &mut Context<Self>) {
        let already_selected = self
            .state
            .selected
            .as_deref()
            .is_some_and(|selected| selected == row_key(target));
        if already_selected {
            return;
        }
        match target {
            RowTarget::Container { id, .. } => self.select_container_item(id.clone()),
            RowTarget::Image(id) => self.select_image_item(id.clone()),
            RowTarget::Volume(name) => self.select_volume_item(name.clone()),
            RowTarget::Network(id) => self.select_network_item(id.clone()),
            RowTarget::Compose(name) => self.select_compose_item(name.clone()),
        }
        // 换条目回到结构化视图：上一个容器的原始 JSON 留在这一条上没有意义。
        self.state.overview_raw = false;
        cx.notify();
    }

    /// 把列表行的动作意向交给外壳统一分发。
    ///
    /// 设计原理（Why）：行内快捷按钮与右键菜单只持有工作区实体句柄，选中、生命周期
    /// 与删除确认等实体动作，以及需要窗口的表单/弹窗，统一由外壳既有分发逻辑完成；
    /// 渲染函数因此不需要 `Context<AppShell>`。事件派发阶段拿不到 `Window`，需要
    /// 窗口的动作由外壳排进 `window_requests`，在渲染期执行。
    pub(crate) fn run_row_action(
        &mut self,
        target: RowTarget,
        action: RowAction,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ContainerWorkspaceEvent::RunRowAction { target, action });
    }

    /// 切换顶部功能分类：清空选中项并拉取该分类列表。
    ///
    /// 设计原理（Why）：分类属于工作区自身状态，切换只需清空输入框并重新拉取，
    /// 不依赖外壳的其它字段；`tab_id` 由调用方传入并在调试期校验，防止多标签
    /// 场景下把分类切换误发到别的实体。
    pub(crate) fn select_container_section(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        debug_assert_eq!(tab_id, self.tab_id, "分类切换必须落在实体所属标签上");
        self.stop_container_streams();
        let create_input = self.state.create_input.clone();
        self.state.apply_section(section);
        create_input.update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh_section();
        cx.notify();
    }

    /// 从编排/卷/网络详情跳到某个容器：必要时先切分类并拉列表。
    ///
    /// 设计原理（Why）：跳转只改本工作区的分类、待选中项与列表，不涉及窗口或
    /// 其它实体，因此归属工作区实体；详情页拿到实体句柄后直接调用，不必再经
    /// 外壳转发。
    pub(crate) fn reveal_container(&mut self, name: String, cx: &mut Context<Self>) {
        let already_visible = self.state.section == ContainerSection::Containers
            && self.state.containers.iter().any(|item| item.name == name);
        let visible_id = self
            .state
            .containers
            .iter()
            .find(|item| item.name == name)
            .map(|item| item.id.clone());
        let need_list =
            self.state.section != ContainerSection::Containers || self.state.containers.is_empty();
        if already_visible {
            if let Some(id) = visible_id {
                self.select_container_item(id);
                cx.notify();
            }
            return;
        }
        self.state.apply_section(ContainerSection::Containers);
        self.state.pending_select = Some(name);
        if need_list {
            self.stop_container_streams();
            self.refresh_section();
        }
        cx.notify();
    }

    /// 把一个容器从指定网络断开。
    ///
    /// 设计原理（Why）：断网只是向控制器投递命令并登记在途请求，动作完全落在
    /// 本实体上；网络详情页直接调用，不再经外壳的 `with_container_mut` 转手。
    pub(crate) fn disconnect_network(&mut self, network: String, container: String) {
        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.send_tracked(
            request_id,
            ContainerCommand::DisconnectNetwork {
                tab_id: self.tab_id,
                request_id,
                network,
                container,
                force: false,
            },
        );
    }
}

impl AppShell {
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
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, cx| workspace.select_row(&target, cx));
        }
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
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, cx| {
                        workspace.container_lifecycle(name, action, cx)
                    });
                }
            }
            RowAction::Remove => {
                let confirm = match target {
                    RowTarget::Container { name, .. } => Some(ContainerConfirm::Container(name)),
                    RowTarget::Image(id) => Some(ContainerConfirm::Image(id)),
                    RowTarget::Volume(name) => Some(ContainerConfirm::Volume(name)),
                    RowTarget::Network(id) => Some(ContainerConfirm::Network(id)),
                    RowTarget::Compose(_) => None,
                };
                if let Some(confirm) = confirm {
                    self.with_container_mut(tab_id, cx, |workspace| {
                        workspace.confirm = Some(confirm);
                    });
                    cx.notify();
                }
            }
            RowAction::Run => {
                let image = match &target {
                    RowTarget::Image(id) => self
                        .state
                        .container_workspace(tab_id)
                        .and_then(|workspace| {
                            let state = &workspace.read(cx).state;
                            state
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
            RowAction::Clone => {
                let RowTarget::Container { id, name } = target else {
                    return;
                };
                self.clone_container(tab_id, &id, &name, window, cx);
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
                        let state = &workspace.read(cx).state;
                        state
                            .images
                            .iter()
                            .find(|item| item.id == id)
                            .map(|item| item.display_name())
                    })
                    .unwrap_or_else(|| id.clone());
                self.open_container_push_dialog(tab_id, id, reference, window, cx);
            }
            RowAction::Save => {
                let RowTarget::Image(id) = target else {
                    return;
                };
                self.save_image(tab_id, id, cx);
            }
            RowAction::History => {
                let RowTarget::Image(id) = target else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.select_image_item(id);
                        workspace.state.detail_tab = ContainerDetailTab::History;
                    });
                }
                cx.notify();
            }
            RowAction::EditComposeYaml => {
                let RowTarget::Compose(name) = target else {
                    return;
                };
                self.edit_compose_yaml(tab_id, &name, window, cx);
            }
            RowAction::BackupVolume => {
                let RowTarget::Volume(name) = target else {
                    return;
                };
                self.backup_volume(tab_id, name, cx);
            }
            RowAction::RestoreVolume => {
                let RowTarget::Volume(name) = target else {
                    return;
                };
                self.restore_volume(tab_id, Some(name), cx);
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
        let Some((previous, logs, stats, terminal)) =
            self.with_container(tab_id, cx, |workspace| {
                (
                    workspace.detail_tab,
                    workspace.logs_request,
                    workspace.stats_request,
                    workspace
                        .terminal_request
                        .as_ref()
                        .map(|(request_id, ..)| *request_id),
                )
            })
        else {
            return;
        };
        self.with_container_mut(tab_id, cx, |workspace| workspace.detail_tab = tab);

        let workspace = self.state.container_workspace(tab_id);
        if previous == ContainerDetailTab::Logs
            && tab != ContainerDetailTab::Logs
            && let Some(request_id) = logs
            && let Some(workspace) = workspace.as_ref()
        {
            workspace.update(cx, |workspace, _| {
                workspace.cancel_container_request(request_id);
                workspace.state.logs_request = None;
                workspace.state.logs_follow = false;
            });
        }
        if previous == ContainerDetailTab::Stats
            && tab != ContainerDetailTab::Stats
            && let Some(request_id) = stats
            && let Some(workspace) = workspace.as_ref()
        {
            workspace.update(cx, |workspace, _| {
                workspace.cancel_container_request(request_id);
                workspace.state.stats_request = None;
            });
        }
        if previous == ContainerDetailTab::Stats && tab != ContainerDetailTab::Stats {
            let processes = self
                .with_container_mut(tab_id, cx, |workspace| {
                    workspace.stats_processes_request.take()
                })
                .flatten();
            if let Some(request_id) = processes
                && let Some(workspace) = workspace.as_ref()
            {
                workspace.update(cx, |workspace, _| {
                    workspace.cancel_container_request(request_id);
                    workspace.state.stats_processes_loading = false;
                });
            }
        }
        // 还没建立完的终端：用户已经切走就撤销，避免后台建好后又把页签抢回去。
        if previous == ContainerDetailTab::Terminal
            && tab != ContainerDetailTab::Terminal
            && let Some(request_id) = terminal
            && let Some(workspace) = workspace.as_ref()
        {
            workspace.update(cx, |workspace, _| {
                workspace.cancel_container_request(request_id);
                workspace.state.terminal_request = None;
            });
        }

        match tab {
            ContainerDetailTab::Logs => {
                if let Some(workspace) = workspace.as_ref() {
                    workspace.update(cx, |workspace, _| workspace.ensure_container_logs());
                }
            }
            ContainerDetailTab::Stats => {
                if let Some(workspace) = workspace.as_ref() {
                    workspace.update(cx, |workspace, _| {
                        workspace.ensure_container_stats();
                        workspace.ensure_container_processes();
                    });
                }
            }
            ContainerDetailTab::Files => {
                if let Some(workspace) = workspace.as_ref() {
                    workspace.update(cx, |workspace, cx| workspace.ensure_container_files(cx));
                }
            }
            ContainerDetailTab::Terminal => self.ensure_container_terminal(tab_id, None, None, cx),
            ContainerDetailTab::Changes => self.ensure_container_changes(tab_id, cx),
            ContainerDetailTab::Overview
            | ContainerDetailTab::History
            | ContainerDetailTab::Yaml => {}
        }
        cx.notify();
    }
}
