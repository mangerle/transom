//! 容器详情区：基础信息、日志、终端、统计与文件预览。

use crate::app::AppShell;
use crate::container::types::{
    ContainerConfirm, ContainerDetailTab, ContainerSection, LOG_TAIL_CHOICES, filtered_log_lines,
};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{
    ComposeProject, ContainerDetails, ContainerFileEntry, ContainerFileKind, ContainerImageDetails,
    ContainerNetworkDetails, ContainerStats, ContainerSummary, ContainerVolumeDetails, TabId,
};

mod changes;
mod compose;
pub(crate) mod compose_graph;
mod compose_topology;
pub(crate) mod compose_yaml;
mod confirm;
mod file_bars;
mod file_preview;
mod files;
mod header;
mod header_actions;
mod header_container;
mod image;
mod image_history;
mod logs;
mod network;
mod overview;
mod overview_cards;
mod raw_inspect;
mod terminal;
pub(crate) mod timeline;
mod volume;

pub(super) use compose::render_compose_detail;
pub(super) use confirm::delete_button;
use file_bars::mode_label;
pub(crate) use file_bars::parent_path;
use files::ContainerFilesView;
pub(super) use files::render_files;
use header::ContainerHeaderView;
pub(super) use header::detail_icon;
pub(crate) use image::render_image_detail;
#[cfg(test)]
pub(crate) use image_history::clean_dockerfile_instruction;
use logs::ContainerLogsView;
pub(super) use logs::render_logs;
pub(super) use network::render_network_detail;
pub(super) use overview::{
    centered_hint, format_unix, kv, linked_containers, render_container_overview, scroll_body,
    short,
};
pub(super) use raw_inspect::render_raw_inspect;
pub(super) use terminal::render_terminal;
pub(super) use volume::render_volume_detail;

impl AppShell {
    /// 右侧详情区。
    pub(crate) fn render_container_detail(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let section = workspace.section;
        let selected = workspace.selected.clone();
        if selected.is_none() {
            let (icon, title_key, desc_key) = match section {
                ContainerSection::Containers => (
                    IconName::Container,
                    "container.empty.containers_title",
                    "container.empty.containers_desc",
                ),
                ContainerSection::Images => (
                    IconName::Layers,
                    "container.empty.images_title",
                    "container.empty.images_desc",
                ),
                ContainerSection::Volumes => (
                    IconName::HardDrive,
                    "container.empty.volumes_title",
                    "container.empty.volumes_desc",
                ),
                ContainerSection::Networks => (
                    IconName::Network,
                    "container.empty.networks_title",
                    "container.empty.networks_desc",
                ),
                ContainerSection::Compose => (
                    IconName::Boxes,
                    "container.empty.compose_title",
                    "container.empty.compose_desc",
                ),
            };
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_6()
                .child(crate::widgets::empty_state(
                    icon,
                    rust_i18n::t!(title_key).to_string(),
                    rust_i18n::t!(desc_key).to_string(),
                    p,
                ))
                .into_any_element();
        }
        let compose_yaml_editor = workspace.compose_yaml_editor.clone();
        let detail_tab = workspace.detail_tab;
        let overview_raw = workspace.overview_raw;
        let browser_host = workspace.browser_host();
        let containers = workspace.containers.clone();
        let details = workspace.details.clone();
        let image_details = workspace.image_details.clone();
        let volume_details = workspace.volume_details.clone();
        let network_details = workspace.network_details.clone();
        let logs = workspace.logs.clone();
        let logs_follow = workspace.logs_follow;
        let logs_tail = workspace.logs_tail;
        let logs_timestamps = workspace.logs_timestamps;
        let logs_reversed = workspace.logs_reversed;
        let logs_search = workspace.logs_search.clone();
        let logs_filter = workspace.logs_search_query(cx);
        let inspect_search = workspace.inspect_search.clone();
        let changes = workspace.changes.clone();
        let changes_loading = workspace.changes_loading;
        let changes_search = workspace.changes_search.clone();
        let stats = workspace.stats.clone();
        let stats_history: Vec<ContainerStats> = workspace.stats_history.iter().cloned().collect();
        let stats_chart = workspace.stats_chart;
        let stats_interval = workspace.stats_interval;
        let stats_processes = workspace.stats_processes.clone();
        let stats_processes_loading = workspace.stats_processes_loading;
        let stats_processes_error = workspace.stats_processes_error.clone();
        let stats_error = workspace.stats_error.clone();
        let stats_ports_collapsed = workspace.stats_ports_collapsed;
        let stats_processes_collapsed = workspace.stats_processes_collapsed;
        let stats_is_running = workspace
            .selected_container()
            .map(|c| c.state.is_running())
            .unwrap_or(false);
        let terminal = workspace
            .terminal
            .as_ref()
            .map(|terminal| terminal.view.clone());
        let terminal_user = workspace
            .terminal
            .as_ref()
            .map(|terminal| terminal.user.clone())
            .or_else(|| {
                workspace
                    .terminal_request
                    .as_ref()
                    .map(|(_, user, _)| user.clone())
            });
        let terminal_shell = workspace
            .terminal
            .as_ref()
            .map(|terminal| terminal.shell.clone())
            .or_else(|| {
                workspace
                    .terminal_request
                    .as_ref()
                    .map(|(_, _, shell)| shell.clone())
            });
        let files = (
            workspace.files.path.clone(),
            workspace.files.entries.clone(),
            workspace.files.preview.clone(),
            workspace.files.preview_editor.clone(),
            workspace.files.preview_saving,
            workspace.files.loading,
            workspace.files.rename_from.clone(),
            workspace.files.rename_input.clone(),
            workspace.files.new_dir_open,
            workspace.files.new_dir_input.clone(),
            workspace.files.scroll.clone(),
        );
        let selected_container = containers
            .iter()
            .find(|item| selected.as_deref() == Some(item.id.as_str()))
            .cloned();
        let selected_image = workspace.selected_image().cloned();
        let selected_volume = workspace.selected_volume().cloned();
        let selected_network = workspace.selected_network().cloned();
        let selected_compose = workspace.selected_compose().cloned();
        let compose_deps = workspace.compose_dependencies.clone();
        let compose_hovered = workspace.compose_hovered_service.clone();
        let compose_deps_is_empty = compose_deps.is_empty();

        let title = match section {
            ContainerSection::Containers => selected_container.as_ref().map(|i| i.name.clone()),
            ContainerSection::Images => selected_image.as_ref().map(|i| i.display_name()),
            ContainerSection::Volumes => selected_volume.as_ref().map(|i| i.name.clone()),
            ContainerSection::Networks => selected_network.as_ref().map(|i| i.name.clone()),
            ContainerSection::Compose => selected_compose.as_ref().map(|i| i.name.clone()),
        }
        .unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string());

        if section == ContainerSection::Compose {
            if let Some(project) = &selected_compose {
                if detail_tab == ContainerDetailTab::Yaml {
                    self.ensure_compose_yaml_loaded(tab_id, &project.name, window, cx);
                } else if compose_deps_is_empty {
                    self.ensure_compose_dependencies_loaded(tab_id, &project.name, cx);
                }
            }
        }

        let tabs = self.render_detail_tab_buttons(tab_id, section, detail_tab, p, cx);
        let header = self.render_detail_header(
            tab_id,
            ContainerHeaderView {
                section,
                title,
                tabs,
                overview_raw,
                detail_tab,
                container: &selected_container,
                details: &details,
                image: &selected_image,
                volume: &selected_volume,
                network: &selected_network,
                compose: &selected_compose,
                all_containers: &containers,
                timeline: if section == ContainerSection::Containers {
                    selected_container
                        .as_ref()
                        .map(|c| timeline::StatusTimeline::infer(details.as_ref(), c.state))
                } else {
                    None
                },
            },
            p,
            cx,
        );

        let content = match (section, detail_tab) {
            (ContainerSection::Containers, ContainerDetailTab::Overview) => {
                render_container_overview(
                    tab_id,
                    details.as_ref(),
                    overview_raw,
                    &inspect_search,
                    &browser_host,
                    p,
                    cx,
                )
            }
            (
                ContainerSection::Containers | ContainerSection::Compose,
                ContainerDetailTab::Logs,
            ) => render_logs(
                ContainerLogsView {
                    tab_id,
                    logs: &logs,
                    follow: logs_follow,
                    tail: logs_tail,
                    timestamps: logs_timestamps,
                    reversed: logs_reversed,
                    search: &logs_search,
                    filter: &logs_filter,
                },
                p,
                cx,
            ),
            (ContainerSection::Containers, ContainerDetailTab::Terminal) => {
                render_terminal(tab_id, terminal, terminal_user, terminal_shell, p, cx)
            }
            (ContainerSection::Containers, ContainerDetailTab::Stats) => {
                super::stats::render_container_stats(
                    super::stats::ContainerStatsView {
                        tab_id,
                        stats: stats.as_ref(),
                        history: &stats_history,
                        chart: stats_chart,
                        interval: stats_interval,
                        error: stats_error.as_deref(),
                        details: details.as_ref(),
                        browser_host: &browser_host,
                        processes: &stats_processes,
                        processes_loading: stats_processes_loading,
                        processes_error: stats_processes_error.as_deref(),
                        is_running: stats_is_running,
                        has_selection: selected.is_some(),
                        ports_collapsed: stats_ports_collapsed,
                        processes_collapsed: stats_processes_collapsed,
                    },
                    p,
                    cx,
                )
            }
            (
                ContainerSection::Containers | ContainerSection::Volumes,
                ContainerDetailTab::Files,
            ) => {
                let is_volume = section == ContainerSection::Volumes;
                render_files(
                    tab_id,
                    ContainerFilesView {
                        path: &files.0,
                        entries: &files.1,
                        preview: files.2.as_ref(),
                        preview_editor: files.3.as_ref(),
                        preview_saving: files.4,
                        loading: files.5,
                        scroll: &files.10,
                        readonly: is_volume,
                    },
                    p,
                    cx,
                )
            }
            (ContainerSection::Containers, ContainerDetailTab::Changes) => {
                changes::render_container_changes(
                    tab_id,
                    selected.as_deref(),
                    changes.as_ref(),
                    changes_loading,
                    &changes_search,
                    p,
                    cx,
                )
            }
            (ContainerSection::Compose, _) => render_compose_detail(
                compose::ComposeDetailParams {
                    tab_id,
                    project: selected_compose.as_ref(),
                    containers: &containers,
                    detail_tab,
                    compose_yaml_editor: compose_yaml_editor.as_ref(),
                    dependencies: &compose_deps,
                    hovered_service: compose_hovered.as_deref(),
                },
                p,
                window,
                cx,
            ),
            (ContainerSection::Images, _) => render_image_detail(
                tab_id,
                selected_image.as_ref(),
                image_details.as_ref(),
                overview_raw,
                detail_tab,
                &inspect_search,
                p,
                cx,
            ),
            (ContainerSection::Volumes, _) => render_volume_detail(
                tab_id,
                selected_volume.as_ref(),
                volume_details.as_ref(),
                overview_raw,
                &inspect_search,
                p,
                cx,
            ),
            (ContainerSection::Networks, _) => render_network_detail(
                tab_id,
                selected_network.as_ref(),
                network_details.as_ref(),
                overview_raw,
                &inspect_search,
                p,
                cx,
            ),
            (ContainerSection::Containers, _) => render_container_overview(
                tab_id,
                details.as_ref(),
                overview_raw,
                &inspect_search,
                &browser_host,
                p,
                cx,
            ),
        };

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
            .into_any_element()
    }

    /// 详情页签按钮组：不含外边距与分隔线，由详情头部把它放在同一行的左端。
    ///
    /// 只有一个页签的分类（镜像/卷/网络/编排）没有可切换的东西，返回 `None`，
    /// 头部因此保留标题。
    fn render_detail_tab_buttons(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        current: ContainerDetailTab,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tabs = section.detail_tabs();
        if tabs.len() <= 1 {
            return None;
        }
        let app = cx.entity().clone();
        // 当前终端会话（含正在建立）的执行用户；`None` 表示还没有会话。
        let current_user = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| {
                workspace
                    .terminal
                    .as_ref()
                    .map(|terminal| terminal.user.clone())
                    .or_else(|| {
                        workspace
                            .terminal_request
                            .as_ref()
                            .map(|(_, user, _)| user.clone())
                    })
            });
        let user_tooltip = match &current_user {
            Some(Some(user)) => {
                rust_i18n::t!("container.terminal.current_user", user = user.as_str()).to_string()
            }
            Some(None) => rust_i18n::t!("container.terminal.current_default_user").to_string(),
            None => rust_i18n::t!("container.terminal.as_user").to_string(),
        };
        Some(
            h_flex()
                .gap_1()
                .items_center()
                .flex_none()
                .min_w_0()
                .children(tabs.iter().copied().map(|tab| {
                    if tab != ContainerDetailTab::Terminal {
                        return Button::new(ElementId::Name(
                            format!("container-detail-tab-{}", tab.label_key()).into(),
                        ))
                        .xsmall()
                        .icon(detail_icon(tab))
                        .label(rust_i18n::t!(tab.label_key()).to_string())
                        .selected(tab == current)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_container_detail_tab(tab_id, tab, window, cx);
                        }))
                        .into_any_element();
                    }
                    // 终端页签带一个下拉：root 与镜像默认用户的会话是两回事，
                    // 点主按钮仍走默认用户，避免改变原有肌肉记忆。主按钮与
                    // 三角共用一个外框，视觉上是一枚分体按钮而不是两枚。
                    h_flex()
                        .items_center()
                        .flex_none()
                        .rounded(p.radius.md)
                        .border_1()
                        .border_color(p.border)
                        .overflow_hidden()
                        .child(
                            Button::new("container-detail-tab-terminal")
                                .xsmall()
                                .ghost()
                                .icon(IconName::Terminal)
                                .label(rust_i18n::t!("container.detail.terminal").to_string())
                                .selected(tab == current)
                                .tooltip(user_tooltip.clone())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_container_detail_tab(tab_id, tab, window, cx);
                                })),
                        )
                        .child(div().w(px(1.0)).h(px(14.0)).bg(p.border))
                        .child(
                            Button::new("container-detail-tab-terminal-menu")
                                .xsmall()
                                .ghost()
                                .icon(IconName::ChevronDown)
                                .tooltip(user_tooltip.clone())
                                .dropdown_menu({
                                    let app = app.clone();
                                    move |menu, _, _| {
                                        terminal::populate_terminal_menu(menu, tab_id, app.clone())
                                    }
                                }),
                        )
                        .into_any_element()
                }))
                .into_any_element(),
        )
    }
}
