//! 容器详情区：基础信息、日志、终端、统计与文件预览。

use crate::app::AppShell;
use crate::container::types::{
    ContainerConfirm, ContainerDetailTab, ContainerSection, LOG_TAIL_CHOICES, log_lines,
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

mod compose;
mod confirm;
mod file_bars;
mod files;
mod header;
mod image;
mod logs;
mod network;
mod overview;
mod terminal;
mod volume;

pub(super) use compose::render_compose_detail;
pub(super) use confirm::{delete_button, render_confirm_bar};
use file_bars::{mode_label, parent_path, render_new_dir_bar, render_rename_bar};
use files::ContainerFilesView;
pub(super) use files::render_files;
use header::ContainerHeaderView;
pub(super) use header::detail_icon;
pub(super) use image::render_image_detail;
use logs::ContainerLogsView;
pub(super) use logs::render_logs;
pub(super) use network::render_network_detail;
pub(super) use overview::{
    centered_hint, format_unix, kv, linked_containers, render_container_overview, scroll_body,
    short,
};
pub(super) use terminal::render_terminal;
pub(super) use volume::render_volume_detail;

impl AppShell {
    /// 容器改名条：与文件重命名同样的形态，避免为一次改名再弹一层模态。
    pub(crate) fn render_container_rename_bar(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<AppShell>,
    ) -> Option<AnyElement> {
        let (_, input) = self
            .state
            .container_workspace(tab_id)?
            .rename_container
            .clone()?;
        Some(
            h_flex()
                .gap_1()
                .px_2()
                .py_1()
                .items_center()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    div()
                        .text_xs()
                        .flex_none()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.rename.label").to_string()),
                )
                .child(Input::new(&input).flex_1().min_w_0())
                .child(
                    Button::new("container-rename-submit")
                        .primary()
                        .xsmall()
                        .icon(IconName::Check)
                        .label(rust_i18n::t!("common.confirm").to_string())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.submit_container_rename(tab_id, window, cx);
                        })),
                )
                .child(
                    Button::new("container-rename-cancel")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_container_rename(tab_id, cx);
                        })),
                )
                .into_any_element(),
        )
    }

    /// 右侧详情区。
    pub(crate) fn render_container_detail(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let section = workspace.section;
        let selected = workspace.selected.clone();
        let confirm = workspace.confirm.clone();
        let detail_tab = workspace.detail_tab;
        let overview_raw = workspace.overview_raw;
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
        let stats = workspace.stats.clone();
        let stats_history: Vec<ContainerStats> = workspace.stats_history.iter().cloned().collect();
        let stats_chart = workspace.stats_chart;
        let stats_interval = workspace.stats_interval;
        let stats_processes = workspace.stats_processes.clone();
        let stats_processes_loading = workspace.stats_processes_loading;
        let stats_processes_error = workspace.stats_processes_error.clone();
        let stats_error = workspace.stats_error.clone();
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
                    .map(|(_, user)| user.clone())
            });
        let files = (
            workspace.files.path.clone(),
            workspace.files.entries.clone(),
            workspace.files.preview.clone(),
            workspace.files.loading,
            workspace.files.rename_from.clone(),
            workspace.files.rename_input.clone(),
            workspace.files.new_dir_open,
            workspace.files.new_dir_input.clone(),
            workspace.files.scroll.clone(),
        );
        let status_text = workspace.notice.clone();

        let selected_container = containers
            .iter()
            .find(|item| selected.as_deref() == Some(item.id.as_str()))
            .cloned();
        let selected_image = workspace.selected_image().cloned();
        let selected_volume = workspace.selected_volume().cloned();
        let selected_network = workspace.selected_network().cloned();
        let selected_compose = workspace.selected_compose().cloned();

        let title = match section {
            ContainerSection::Containers => selected_container
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string()),
            ContainerSection::Images => selected_image
                .as_ref()
                .map(|item| item.display_name())
                .unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string()),
            ContainerSection::Volumes => selected_volume
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string()),
            ContainerSection::Networks => selected_network
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string()),
            ContainerSection::Compose => selected_compose
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string()),
        };

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
            },
            p,
            cx,
        );
        let confirm_bar = confirm.map(|confirm| render_confirm_bar(tab_id, confirm, p, cx));
        let rename_bar = self.render_container_rename_bar(tab_id, p, cx);

        let content = match (section, detail_tab) {
            (ContainerSection::Containers, ContainerDetailTab::Overview) => {
                render_container_overview(details.as_ref(), status_text.as_deref(), overview_raw, p)
            }
            (ContainerSection::Containers, ContainerDetailTab::Logs) => render_logs(
                ContainerLogsView {
                    tab_id,
                    logs: &logs,
                    follow: logs_follow,
                    tail: logs_tail,
                    timestamps: logs_timestamps,
                    reversed: logs_reversed,
                },
                p,
                cx,
            ),
            (ContainerSection::Containers, ContainerDetailTab::Terminal) => {
                render_terminal(tab_id, terminal, terminal_user, p, cx)
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
                        processes: &stats_processes,
                        processes_loading: stats_processes_loading,
                        processes_error: stats_processes_error.as_deref(),
                        has_selection: selected.is_some(),
                    },
                    p,
                    cx,
                )
            }
            (ContainerSection::Containers, ContainerDetailTab::Files) => render_files(
                tab_id,
                ContainerFilesView {
                    path: &files.0,
                    entries: &files.1,
                    preview: files.2.as_ref(),
                    loading: files.3,
                    rename_from: files.4.as_deref(),
                    rename_input: files.5.as_ref(),
                    new_dir_open: files.6,
                    new_dir_input: files.7.as_ref(),
                    scroll: &files.8,
                },
                p,
                cx,
            ),
            (ContainerSection::Compose, _) => {
                render_compose_detail(tab_id, selected_compose.as_ref(), &containers, p, cx)
            }
            (ContainerSection::Images, _) => {
                render_image_detail(selected_image.as_ref(), image_details.as_ref(), p)
            }
            (ContainerSection::Volumes, _) => render_volume_detail(
                tab_id,
                selected_volume.as_ref(),
                volume_details.as_ref(),
                p,
                cx,
            ),
            (ContainerSection::Networks, _) => render_network_detail(
                tab_id,
                selected_network.as_ref(),
                network_details.as_ref(),
                p,
                cx,
            ),
        };

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .children(rename_bar)
            .children(confirm_bar)
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
                            .map(|(_, user)| user.clone())
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
                    let root_app = app.clone();
                    let default_app = app.clone();
                    let menu_user = current_user.clone();
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
                                .dropdown_menu(move |menu, _, _| {
                                    let root_app = root_app.clone();
                                    let default_app = default_app.clone();
                                    menu.item(
                                        PopupMenuItem::new(
                                            rust_i18n::t!("container.terminal.as_root").to_string(),
                                        )
                                        .icon(IconName::Terminal)
                                        .checked(
                                            matches!(&menu_user, Some(Some(user)) if user == "root"),
                                        )
                                        .on_click(move |_, _, cx| {
                                            let app = root_app.clone();
                                            app.update(cx, |this, cx| {
                                                this.open_container_terminal_as(
                                                    tab_id,
                                                    Some("root".to_string()),
                                                    cx,
                                                );
                                            });
                                        }),
                                    )
                                    .item(
                                        PopupMenuItem::new(
                                            rust_i18n::t!("container.terminal.as_default")
                                                .to_string(),
                                        )
                                        .icon(IconName::Terminal)
                                        .checked(matches!(&menu_user, Some(None)))
                                        .on_click(move |_, _, cx| {
                                            let app = default_app.clone();
                                            app.update(cx, |this, cx| {
                                                this.open_container_terminal_as(tab_id, None, cx);
                                            });
                                        }),
                                    )
                                }),
                        )
                        .into_any_element()
                }))
                .into_any_element(),
        )
    }
}
