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
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{
    ComposeProject, ContainerDetails, ContainerFileEntry, ContainerFileKind, ContainerImageDetails,
    ContainerNetworkDetails, ContainerStats, ContainerSummary, ContainerVolumeDetails, TabId,
};

impl AppShell {
    /// 右侧详情区。
    pub(crate) fn render_container_detail(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let section = workspace.section;
        let selected = workspace.selected.clone();
        let confirm = workspace.confirm.clone();
        let detail_tab = workspace.detail_tab;
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
        let workspace_stats_enabled = workspace.stats_enabled;
        let terminal = workspace
            .terminal
            .as_ref()
            .map(|terminal| terminal.view.clone());
        let files = (
            workspace.files.path.clone(),
            workspace.files.entries.clone(),
            workspace.files.preview.clone(),
            workspace.files.loading,
            workspace.files.rename_from.clone(),
            workspace.files.rename_input.clone(),
            workspace.files.new_dir_open,
            workspace.files.new_dir_input.clone(),
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

        let header = self.render_detail_header(
            tab_id,
            ContainerHeaderView {
                section,
                title,
                container: &selected_container,
                image: &selected_image,
                volume: &selected_volume,
                network: &selected_network,
                compose: &selected_compose,
                all_containers: &containers,
            },
            p,
            cx,
        );
        let tabs = self.render_detail_tabs(tab_id, section, detail_tab, p, cx);
        let confirm_bar = confirm.map(|confirm| render_confirm_bar(tab_id, confirm, p, cx));

        let content = match (section, detail_tab) {
            (ContainerSection::Containers, ContainerDetailTab::Overview) => {
                render_container_overview(details.as_ref(), status_text.as_deref(), p)
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
                render_terminal(tab_id, terminal, p, cx)
            }
            (ContainerSection::Containers, ContainerDetailTab::Stats) => {
                render_stats(tab_id, stats.as_ref(), workspace_stats_enabled, p, cx)
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
            .children(tabs)
            .children(confirm_bar)
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
            .into_any_element()
    }

    fn render_detail_header(
        &mut self,
        tab_id: TabId,
        view: ContainerHeaderView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ContainerHeaderView {
            section,
            title,
            container,
            image,
            volume,
            network,
            compose,
            all_containers,
        } = view;
        let mut actions = h_flex().gap_1().items_center().flex_shrink_0();
        match section {
            ContainerSection::Containers => {
                if let Some(container) = container {
                    let name = container.name.clone();
                    let running = container.state.is_running();
                    if running {
                        actions = actions
                            .child(
                                Button::new("container-stop")
                                    .xsmall()
                                    .icon(IconName::Square)
                                    .label(rust_i18n::t!("container.action.stop").to_string())
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |this, _, _, cx| {
                                            this.container_lifecycle(
                                                tab_id,
                                                name.clone(),
                                                transom_core::ContainerLifecycleAction::Stop,
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(
                                Button::new("container-restart")
                                    .xsmall()
                                    .icon(IconName::RefreshCw)
                                    .label(rust_i18n::t!("container.action.restart").to_string())
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |this, _, _, cx| {
                                            this.container_lifecycle(
                                                tab_id,
                                                name.clone(),
                                                transom_core::ContainerLifecycleAction::Restart,
                                                cx,
                                            );
                                        }
                                    })),
                            );
                    } else {
                        actions = actions.child(
                            Button::new("container-start")
                                .xsmall()
                                .icon(IconName::Play)
                                .label(rust_i18n::t!("container.action.start").to_string())
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, _, cx| {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Start,
                                            cx,
                                        );
                                    }
                                })),
                        );
                    }
                    actions = actions
                        .child(
                            Button::new("container-open-terminal")
                                .xsmall()
                                .icon(IconName::Terminal)
                                .label(rust_i18n::t!("container.detail.terminal").to_string())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_container_detail_tab(
                                        tab_id,
                                        ContainerDetailTab::Terminal,
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("container-remove")
                                .danger()
                                .xsmall()
                                .icon(IconName::Trash)
                                .label(rust_i18n::t!("common.delete").to_string())
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, _, cx| {
                                        if let Some(workspace) =
                                            this.container_workspace_mut(tab_id)
                                        {
                                            workspace.confirm =
                                                Some(ContainerConfirm::Container(name.clone()));
                                        }
                                        cx.notify();
                                    }
                                })),
                        );
                }
            }
            ContainerSection::Compose => {
                if let Some(project) = compose {
                    let names: Vec<String> = all_containers
                        .iter()
                        .filter(|item| {
                            item.compose_project.as_deref() == Some(project.name.as_str())
                        })
                        .map(|item| item.name.clone())
                        .collect();
                    let start_names = names.clone();
                    let stop_names = names;
                    actions = actions
                        .child(
                            Button::new("compose-start-all")
                                .xsmall()
                                .icon(IconName::Play)
                                .label(rust_i18n::t!("container.action.start_all").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    for name in &start_names {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Start,
                                            cx,
                                        );
                                    }
                                })),
                        )
                        .child(
                            Button::new("compose-stop-all")
                                .xsmall()
                                .icon(IconName::Square)
                                .label(rust_i18n::t!("container.action.stop_all").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    for name in &stop_names {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Stop,
                                            cx,
                                        );
                                    }
                                })),
                        );
                }
            }
            ContainerSection::Images => {
                if let Some(image) = image {
                    actions = actions.child(delete_button(
                        "image-delete",
                        cx,
                        tab_id,
                        ContainerConfirm::Image(image.id.clone()),
                    ));
                }
            }
            ContainerSection::Volumes => {
                if let Some(volume) = volume {
                    actions = actions.child(delete_button(
                        "volume-delete",
                        cx,
                        tab_id,
                        ContainerConfirm::Volume(volume.name.clone()),
                    ));
                }
            }
            ContainerSection::Networks => {
                if let Some(network) = network {
                    actions = actions.child(delete_button(
                        "network-delete",
                        cx,
                        tab_id,
                        ContainerConfirm::Network(network.id.clone()),
                    ));
                }
            }
        }

        h_flex()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .flex_none()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .child(Icon::new(section.icon()).small().text_color(p.primary))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(title),
                    ),
            )
            .child(actions)
            .into_any_element()
    }

    fn render_detail_tabs(
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
        Some(
            h_flex()
                .px_3()
                .py_1()
                .gap_1()
                .flex_none()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .children(tabs.iter().copied().map(|tab| {
                    Button::new(ElementId::Name(
                        format!("container-detail-tab-{}", tab.label_key()).into(),
                    ))
                    .xsmall()
                    .icon(detail_icon(tab))
                    .label(rust_i18n::t!(tab.label_key()).to_string())
                    .selected(tab == current)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_container_detail_tab(tab_id, tab, window, cx);
                    }))
                }))
                .into_any_element(),
        )
    }
}

fn detail_icon(tab: ContainerDetailTab) -> IconName {
    match tab {
        ContainerDetailTab::Overview => IconName::Info,
        ContainerDetailTab::Logs => IconName::ScrollText,
        ContainerDetailTab::Terminal => IconName::Terminal,
        ContainerDetailTab::Stats => IconName::Gauge,
        ContainerDetailTab::Files => IconName::FolderOpen,
    }
}

fn delete_button(
    id: &'static str,
    cx: &mut Context<AppShell>,
    tab_id: TabId,
    confirm: ContainerConfirm,
) -> AnyElement {
    Button::new(id)
        .danger()
        .xsmall()
        .icon(IconName::Trash)
        .label(rust_i18n::t!("common.delete").to_string())
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(workspace) = this.container_workspace_mut(tab_id) {
                workspace.confirm = Some(confirm.clone());
            }
            cx.notify();
        }))
        .into_any_element()
}

fn render_confirm_bar(
    tab_id: TabId,
    confirm: ContainerConfirm,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (label, recursive) = match &confirm {
        ContainerConfirm::Container(name) => (
            rust_i18n::t!("container.confirm.remove_container", name = name).to_string(),
            false,
        ),
        ContainerConfirm::Image(image) => (
            rust_i18n::t!("container.confirm.remove_image", name = short(image)).to_string(),
            false,
        ),
        ContainerConfirm::Volume(name) => (
            rust_i18n::t!("container.confirm.remove_volume", name = name).to_string(),
            false,
        ),
        ContainerConfirm::Network(name) => (
            rust_i18n::t!("container.confirm.remove_network", name = name).to_string(),
            false,
        ),
        ContainerConfirm::FilePath { path, recursive } => (
            rust_i18n::t!("container.confirm.remove_path", path = path).to_string(),
            *recursive,
        ),
    };
    h_flex()
        .mx_3()
        .my_2()
        .p_2()
        .gap_3()
        .items_center()
        .justify_between()
        .rounded(p.radius.md)
        .bg(p.danger.opacity(0.12))
        .border_1()
        .border_color(p.danger.opacity(0.4))
        .child(div().text_sm().text_color(p.danger).child(label))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("container-confirm-cancel")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_container_confirm(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("container-confirm-delete")
                        .danger()
                        .xsmall()
                        .icon(if recursive {
                            IconName::Trash
                        } else {
                            IconName::Check
                        })
                        .label(rust_i18n::t!("common.confirm_delete").to_string())
                        .on_click(cx.listener({
                            let confirm = confirm.clone();
                            move |this, _, _, cx| {
                                this.apply_container_confirm(tab_id, confirm.clone(), cx);
                            }
                        })),
                ),
        )
        .into_any_element()
}

fn render_container_overview(
    details: Option<&ContainerDetails>,
    notice: Option<&str>,
    p: &Palette,
) -> AnyElement {
    let Some(details) = details else {
        return centered_hint(rust_i18n::t!("container.detail.loading").to_string(), p);
    };
    let mut rows = v_flex().gap_1p5();
    rows = rows
        .child(kv(
            p,
            rust_i18n::t!("container.field.id"),
            short(&details.id),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.image"),
            details.image.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.state"),
            details.status.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.command"),
            details.command.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.entrypoint"),
            details.entrypoint.join(" "),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.user"),
            details.user.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.working_dir"),
            details.working_dir.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.restart"),
            details.restart_policy.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.created"),
            format_unix(details.created),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.started_at"),
            details.started_at.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.exit_code"),
            details
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_default(),
        ));
    let ports = details
        .ports
        .iter()
        .map(|port| match port.public_port {
            Some(public) => format!("{public}->{}/{}", port.private_port, port.protocol),
            None => format!("{}/{}", port.private_port, port.protocol),
        })
        .collect::<Vec<_>>()
        .join(", ");
    rows = rows.child(kv(p, rust_i18n::t!("container.field.ports"), ports));
    let mounts = details
        .mounts
        .iter()
        .map(|mount| format!("{} -> {}", mount.source, mount.destination))
        .collect::<Vec<_>>()
        .join("\n");
    rows = rows.child(kv(p, rust_i18n::t!("container.field.mounts"), mounts));
    if !details.env.is_empty() {
        rows = rows.child(kv(
            p,
            rust_i18n::t!("container.field.env"),
            details.env.join("\n"),
        ));
    }
    if !details.inspect.is_empty() {
        rows = rows.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.field.inspect").to_string()),
                )
                .child(
                    div()
                        .id("container-inspect-scroll")
                        .max_h(px(260.0))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded(p.radius.md)
                        .bg(p.muted.opacity(0.3))
                        .font_family(p.mono_font_family.clone())
                        .text_size(p.mono_font_size)
                        .child(details.inspect.clone()),
                ),
        );
    }
    if let Some(notice) = notice {
        rows = rows.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(notice.to_string()),
        );
    }
    scroll_body(rows, p).into_any_element()
}

/// 关联容器行：卷、网络与编排详情里点名字直接跳到那个容器。
///
/// 设计原理（Why）：这三类详情天然带「谁在用我」的关系，原先只把名字拼成一段
/// 文本，用户看到也只能再回列表用搜索找；跳转由 `reveal_container` 负责切分类与
/// 延迟选中，因此这里只需要一个按钮。
fn linked_containers(
    tab_id: TabId,
    title: String,
    items: Vec<(String, String)>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut block = v_flex()
        .gap_0p5()
        .child(div().text_xs().text_color(p.muted_foreground).child(title));
    if items.is_empty() {
        block = block.child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.linked.none").to_string()),
        );
    }
    for (name, hint) in items {
        let reveal_name = name.clone();
        let mut row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Icon::new(IconName::Container)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(
                Button::new(ElementId::Name(
                    format!("container-link-{name}-{tab_id}").into(),
                ))
                .ghost()
                .xsmall()
                .label(name)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reveal_container(tab_id, reveal_name.clone(), cx);
                })),
            );
        if !hint.is_empty() {
            row = row.child(div().text_xs().text_color(p.muted_foreground).child(hint));
        }
        block = block.child(row);
    }
    block.into_any_element()
}

fn render_compose_detail(
    tab_id: TabId,
    project: Option<&ComposeProject>,
    containers: &[ContainerSummary],
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(project) = project else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };
    let members: Vec<&ContainerSummary> = containers
        .iter()
        .filter(|item| item.compose_project.as_deref() == Some(project.name.as_str()))
        .collect();
    let mut rows = v_flex().gap_1p5();
    rows = rows
        .child(kv(
            p,
            rust_i18n::t!("container.field.state"),
            project.state_label(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.working_dir"),
            project.working_dir.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.config_files"),
            project.config_files.join("\n"),
        ));
    if members.is_empty() {
        rows = rows.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.compose.no_containers").to_string()),
        );
    }
    let member_items = members
        .iter()
        .map(|member| {
            (
                member.name.clone(),
                format!("{} · {}", member.state.key(), member.status),
            )
        })
        .collect();
    rows = rows.child(linked_containers(
        tab_id,
        rust_i18n::t!("container.field.containers").to_string(),
        member_items,
        p,
        cx,
    ));
    scroll_body(rows, p).into_any_element()
}

fn render_image_detail(
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    p: &Palette,
) -> AnyElement {
    if let Some(details) = details {
        let rows = v_flex()
            .gap_1p5()
            .child(kv(
                p,
                rust_i18n::t!("container.field.id"),
                details.id.clone(),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.tags"),
                details.tags.join("\n"),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.size"),
                crate::format::format_size(details.size),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.created"),
                format_unix(details.created),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.os_arch"),
                format!(
                    "{}/{}",
                    details.os.clone().unwrap_or_default(),
                    details.architecture.clone().unwrap_or_default()
                ),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.layers"),
                details.layers.to_string(),
            ));
        return scroll_body(rows, p).into_any_element();
    }
    match summary {
        Some(summary) => {
            let rows = v_flex()
                .gap_1p5()
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.id"),
                    summary.id.clone(),
                ))
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.tags"),
                    summary.tags.join("\n"),
                ))
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.size"),
                    crate::format::format_size(summary.size),
                ));
            scroll_body(rows, p).into_any_element()
        }
        None => centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        ),
    }
}

fn render_volume_detail(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerVolumeSummary>,
    details: Option<&ContainerVolumeDetails>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (name, driver, mountpoint, created, labels, used_by) = if let Some(details) = details {
        (
            details.summary.name.clone(),
            details.summary.driver.clone(),
            details.summary.mountpoint.clone(),
            details.summary.created.clone().unwrap_or_default(),
            details.summary.labels.clone(),
            details.used_by.clone(),
        )
    } else if let Some(summary) = summary {
        (
            summary.name.clone(),
            summary.driver.clone(),
            summary.mountpoint.clone(),
            summary.created.clone().unwrap_or_default(),
            summary.labels.clone(),
            Vec::new(),
        )
    } else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };
    let labels = labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let rows = v_flex()
        .gap_1p5()
        .child(kv(p, rust_i18n::t!("container.field.name"), name))
        .child(kv(p, rust_i18n::t!("container.field.driver"), driver))
        .child(kv(
            p,
            rust_i18n::t!("container.field.mountpoint"),
            mountpoint,
        ))
        .child(kv(p, rust_i18n::t!("container.field.created"), created))
        .child(kv(p, rust_i18n::t!("container.field.labels"), labels))
        .child(linked_containers(
            tab_id,
            rust_i18n::t!("container.field.used_by").to_string(),
            used_by
                .into_iter()
                .map(|name| (name, String::new()))
                .collect(),
            p,
            cx,
        ));
    scroll_body(rows, p).into_any_element()
}

fn render_network_detail(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerNetworkSummary>,
    details: Option<&ContainerNetworkDetails>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (id, name, driver, scope, subnet, gateway, connected) = if let Some(details) = details {
        (
            details.summary.id.clone(),
            details.summary.name.clone(),
            details.summary.driver.clone(),
            details.summary.scope.clone(),
            details.summary.subnet.clone().unwrap_or_default(),
            details.gateway.clone().unwrap_or_default(),
            details.connected.clone(),
        )
    } else if let Some(summary) = summary {
        (
            summary.id.clone(),
            summary.name.clone(),
            summary.driver.clone(),
            summary.scope.clone(),
            summary.subnet.clone().unwrap_or_default(),
            String::new(),
            Vec::new(),
        )
    } else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };
    let connected_items = connected
        .iter()
        .map(|item| (item.name.clone(), item.ipv4.clone().unwrap_or_default()))
        .collect();
    let rows = v_flex()
        .gap_1p5()
        .child(kv(p, rust_i18n::t!("container.field.id"), id))
        .child(kv(p, rust_i18n::t!("container.field.name"), name))
        .child(kv(p, rust_i18n::t!("container.field.driver"), driver))
        .child(kv(p, rust_i18n::t!("container.field.scope"), scope))
        .child(kv(p, rust_i18n::t!("container.field.subnet"), subnet))
        .child(kv(p, rust_i18n::t!("container.field.gateway"), gateway))
        .child(linked_containers(
            tab_id,
            rust_i18n::t!("container.field.connected").to_string(),
            connected_items,
            p,
            cx,
        ));
    scroll_body(rows, p).into_any_element()
}

/// 日志页签渲染参数。
struct ContainerLogsView<'a> {
    tab_id: TabId,
    logs: &'a str,
    follow: bool,
    tail: usize,
    timestamps: bool,
    reversed: bool,
}

fn render_logs(view: ContainerLogsView<'_>, p: &Palette, cx: &mut Context<AppShell>) -> AnyElement {
    let ContainerLogsView {
        tab_id,
        logs,
        follow,
        tail,
        timestamps,
        reversed,
    } = view;
    let mut tails = h_flex().gap_0p5().items_center();
    for choice in LOG_TAIL_CHOICES {
        tails = tails.child(
            Button::new(ElementId::Name(
                format!("container-logs-tail-{choice}").into(),
            ))
            .ghost()
            .xsmall()
            .selected(choice == tail)
            .label(if choice == 0 {
                rust_i18n::t!("container.logs.tail_all").to_string()
            } else {
                choice.to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_container_logs_tail(tab_id, choice);
                cx.notify();
            })),
        );
    }
    let toolbar = h_flex()
        .gap_2()
        .p_2()
        .items_center()
        .child(tails)
        .child(
            Button::new("container-logs-timestamps")
                .ghost()
                .xsmall()
                .icon(IconName::Clock)
                .label(rust_i18n::t!("container.logs.timestamps").to_string())
                .selected(timestamps)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_container_logs_timestamps(tab_id);
                    cx.notify();
                })),
        )
        .child(
            Button::new("container-logs-reverse")
                .ghost()
                .xsmall()
                .icon(IconName::ArrowDown)
                .label(rust_i18n::t!("container.logs.reverse").to_string())
                .selected(reversed)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
                        workspace.logs_reversed = !workspace.logs_reversed;
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("container-logs-copy")
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .label(rust_i18n::t!("container.logs.copy").to_string())
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.copy_container_logs(tab_id, cx);
                })),
        )
        .child(
            Button::new("container-logs-follow")
                .xsmall()
                .icon(IconName::ScrollText)
                .label(rust_i18n::t!("container.logs.follow").to_string())
                .selected(follow)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_container_logs_follow(tab_id, !follow);
                    cx.notify();
                })),
        )
        .child(
            Button::new("container-logs-clear")
                .ghost()
                .xsmall()
                .icon(IconName::Trash)
                .label(rust_i18n::t!("container.logs.clear").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
                        workspace.logs.clear();
                    }
                    cx.notify();
                })),
        );
    let body = if logs.is_empty() {
        centered_hint(rust_i18n::t!("container.logs.empty").to_string(), p)
    } else {
        // 逐行渲染而不是整块文本：反转顺序时才能保持每行独立可选。
        let rows = v_flex().gap_0().children(
            log_lines(logs, reversed)
                .into_iter()
                .map(|line| div().child(line.to_string())),
        );
        div()
            .id("container-logs-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .font_family(p.mono_font_family.clone())
            .text_size(p.mono_font_size)
            .line_height(relative(1.35))
            .child(rows)
            .into_any_element()
    };
    v_flex()
        .size_full()
        .overflow_hidden()
        .child(toolbar)
        .child(body)
        .into_any_element()
}

fn render_terminal(
    tab_id: TabId,
    terminal: Option<Entity<transom_terminal::TerminalView>>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if let Some(terminal) = terminal {
        return div().size_full().child(terminal).into_any_element();
    }
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.terminal.hint").to_string()),
        )
        .child(
            Button::new("container-terminal-open")
                .primary()
                .small()
                .icon(IconName::Terminal)
                .label(rust_i18n::t!("container.terminal.open").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_container_detail_tab(tab_id, ContainerDetailTab::Terminal, window, cx);
                })),
        )
        .into_any_element()
}

fn render_stats(
    tab_id: TabId,
    stats: Option<&ContainerStats>,
    enabled: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let toolbar = h_flex().gap_1().p_2().items_center().child(
        Button::new("container-stats-auto")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .label(rust_i18n::t!("container.stats.auto").to_string())
            .selected(enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_container_stats(tab_id, cx);
            })),
    );
    let Some(stats) = stats else {
        return v_flex()
            .size_full()
            .child(toolbar)
            .child(centered_hint(
                rust_i18n::t!("container.stats.waiting").to_string(),
                p,
            ))
            .into_any_element();
    };
    let rows = v_flex()
        .gap_3()
        .child(stat_bar(
            rust_i18n::t!("container.stats.cpu").to_string(),
            format!("{:.2}%", stats.cpu_percent),
            (stats.cpu_percent / 100.0).clamp(0.0, 1.0),
            p,
        ))
        .child(stat_bar(
            rust_i18n::t!("container.stats.memory").to_string(),
            format!(
                "{} / {}（{:.1}%）",
                crate::format::format_size(stats.memory_used),
                crate::format::format_size(stats.memory_limit),
                stats.memory_percent
            ),
            (stats.memory_percent / 100.0).clamp(0.0, 1.0),
            p,
        ))
        .child(stat_line(
            p,
            rust_i18n::t!("container.stats.network").to_string(),
            format!(
                "{} ↓ / {} ↑",
                crate::format::format_size(stats.network_rx),
                crate::format::format_size(stats.network_tx)
            ),
        ))
        .child(stat_line(
            p,
            rust_i18n::t!("container.stats.block").to_string(),
            format!(
                "{} R / {} W",
                crate::format::format_size(stats.block_read),
                crate::format::format_size(stats.block_write)
            ),
        ))
        .child(stat_line(
            p,
            rust_i18n::t!("container.stats.pids").to_string(),
            stats.pids.to_string(),
        ));
    v_flex()
        .size_full()
        .child(toolbar)
        .child(scroll_body(rows, p))
        .into_any_element()
}

fn stat_bar(label: String, value: String, ratio: f32, p: &Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_sm().child(label))
                .child(div().text_sm().text_color(p.muted_foreground).child(value)),
        )
        .child(
            div().h(px(6.0)).w_full().rounded_full().bg(p.muted).child(
                div()
                    .h_full()
                    .w(relative(ratio))
                    .rounded_full()
                    .bg(p.primary),
            ),
        )
        .into_any_element()
}

fn stat_line(p: &Palette, label: String, value: String) -> AnyElement {
    h_flex()
        .justify_between()
        .child(div().text_sm().child(label))
        .child(div().text_sm().text_color(p.muted_foreground).child(value))
        .into_any_element()
}

fn render_rename_bar(
    tab_id: TabId,
    rename_from: Option<&str>,
    rename_input: Option<&Entity<InputState>>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let from = rename_from?.to_string();
    let input = rename_input?.clone();
    Some(
        h_flex()
            .gap_2()
            .p_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.4))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.files.rename").to_string()),
            )
            .child(Input::new(&input).flex_1())
            .child(
                Button::new("container-file-rename-confirm")
                    .primary()
                    .xsmall()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.confirm").to_string())
                    .on_click(cx.listener({
                        let input = input.clone();
                        let from = from.clone();
                        move |this, _, _, cx| {
                            let value = input.read(cx).value().trim().to_string();
                            if value.is_empty() || value.contains('/') {
                                if let Some(workspace) = this.container_workspace_mut(tab_id) {
                                    workspace.notice = Some(
                                        rust_i18n::t!("container.files.rename_invalid").to_string(),
                                    );
                                }
                                cx.notify();
                                return;
                            }
                            let parent = parent_path(&from);
                            let to = if parent == "/" {
                                format!("/{value}")
                            } else {
                                format!("{parent}/{value}")
                            };
                            this.rename_container_path(tab_id, from.clone(), to);
                            if let Some(workspace) = this.container_workspace_mut(tab_id) {
                                workspace.files.rename_input = None;
                            }
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new("container-file-rename-cancel")
                    .ghost()
                    .xsmall()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(workspace) = this.container_workspace_mut(tab_id) {
                            workspace.files.rename_from = None;
                        }
                        cx.notify();
                    })),
            )
            .into_any_element(),
    )
}

/// 详情头部的渲染参数：选中项按分类不同，统一用借用结构体传入。
struct ContainerHeaderView<'a> {
    section: ContainerSection,
    title: String,
    container: &'a Option<ContainerSummary>,
    image: &'a Option<transom_core::ContainerImageSummary>,
    volume: &'a Option<transom_core::ContainerVolumeSummary>,
    network: &'a Option<transom_core::ContainerNetworkSummary>,
    compose: &'a Option<ComposeProject>,
    all_containers: &'a [ContainerSummary],
}

/// 文件页签的渲染参数：字段多且都是借用，收敛为结构体避免长参数列表。
struct ContainerFilesView<'a> {
    path: &'a str,
    entries: &'a [ContainerFileEntry],
    preview: Option<&'a (String, String, bool)>,
    loading: bool,
    rename_from: Option<&'a str>,
    rename_input: Option<&'a Entity<InputState>>,
    new_dir_open: bool,
    new_dir_input: Option<&'a Entity<InputState>>,
}

/// 「新建目录」条：在当前目录下建子目录，上传与整理都要先有目录。
fn render_new_dir_bar(
    tab_id: TabId,
    input: Option<&Entity<InputState>>,
    _p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(input) = input.cloned() else {
        return div().into_any_element();
    };
    h_flex()
        .gap_1()
        .items_center()
        .child(Input::new(&input).flex_1().min_w_0())
        .child(
            Button::new("container-file-mkdir-confirm")
                .primary()
                .xsmall()
                .icon(IconName::Plus)
                .label(rust_i18n::t!("container.create").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.submit_container_mkdir(tab_id, window, cx);
                })),
        )
        .child(
            Button::new("container-file-mkdir-cancel")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .label(rust_i18n::t!("common.cancel").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_container_mkdir(tab_id, window, cx);
                })),
        )
        .into_any_element()
}

/// tar 头里的权限位转成常见的 `rwxr-xr-x` 形式，只保留读写执行九位。
fn mode_label(mode: u32) -> String {
    const BITS: [(char, u32); 9] = [
        ('r', 0o400),
        ('w', 0o200),
        ('x', 0o100),
        ('r', 0o040),
        ('w', 0o020),
        ('x', 0o010),
        ('r', 0o004),
        ('w', 0o002),
        ('x', 0o001),
    ];
    let mut text = String::with_capacity(9);
    for (glyph, bit) in BITS {
        text.push(if mode & bit != 0 { glyph } else { '-' });
    }
    text
}

fn render_files(
    tab_id: TabId,
    view: ContainerFilesView<'_>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let ContainerFilesView {
        path,
        entries,
        preview,
        loading,
        rename_from,
        rename_input,
        new_dir_open,
        new_dir_input,
    } = view;
    let current_path = if path.is_empty() { "/" } else { path };
    let parent = parent_path(current_path);
    let mut crumbs = h_flex().gap_1().items_center().child(
        Button::new("container-file-up")
            .ghost()
            .xsmall()
            .icon(IconName::ArrowLeft)
            .on_click(cx.listener({
                let parent = parent.clone();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, parent.clone());
                    cx.notify();
                }
            })),
    );
    crumbs = crumbs.child(
        div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(current_path.to_string()),
    );
    crumbs = crumbs.child(div().flex_1()).child(
        Button::new("container-file-refresh")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .on_click(cx.listener({
                let path = current_path.to_string();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, path.clone());
                    cx.notify();
                }
            })),
    );
    crumbs = crumbs.child(
        Button::new("container-file-upload")
            .ghost()
            .xsmall()
            .icon(IconName::Upload)
            .label(rust_i18n::t!("container.files.upload").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.upload_container_file(tab_id);
                cx.notify();
            })),
    );
    crumbs = crumbs.child(
        Button::new("container-file-mkdir")
            .ghost()
            .xsmall()
            .icon(IconName::FolderPlus)
            .label(rust_i18n::t!("container.files.new_dir").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_container_mkdir(tab_id, window, cx);
            })),
    );

    let rename_bar = render_rename_bar(tab_id, rename_from, rename_input, p, cx);
    let mkdir_bar = new_dir_open.then(|| render_new_dir_bar(tab_id, new_dir_input, p, cx));

    // 目录优先、再按名称：归档接口按 tar 顺序返回，用户习惯是先看目录。
    let mut ordered: Vec<&ContainerFileEntry> = entries.iter().collect();
    ordered.sort_by(|left, right| {
        let left_dir = left.kind == ContainerFileKind::Directory;
        let right_dir = right.kind == ContainerFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut list = v_flex().gap_0p5();
    if loading {
        list = list.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.loading").to_string()),
        );
    }
    for entry in ordered {
        let entry_path = entry.path.clone();
        let entry_name = entry.name.clone();
        let is_dir = entry.kind == ContainerFileKind::Directory;
        let name = entry.name.clone();
        let mut row = h_flex()
            .id(ElementId::Name(
                format!("container-file-row-{entry_path}").into(),
            ))
            .gap_2()
            .px_2()
            .py_1()
            .rounded(p.radius.md)
            .items_center()
            .hover(|style| style.bg(p.muted.opacity(0.5)))
            .child(
                Icon::new(if is_dir {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small()
                .text_color(p.muted_foreground),
            )
            .child(div().flex_1().min_w_0().text_sm().truncate().child(name))
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .text_color(p.muted_foreground)
                    .child(mode_label(entry.mode)),
            )
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .text_color(p.muted_foreground)
                    .child(entry.modified.map(format_unix).unwrap_or_default()),
            )
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .w(px(64.0))
                    .text_right()
                    .text_color(p.muted_foreground)
                    .child(if is_dir {
                        String::new()
                    } else {
                        crate::format::format_size(entry.size)
                    }),
            );
        if is_dir {
            row = row.on_click(cx.listener({
                let entry_path = entry_path.clone();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, entry_path.clone());
                    cx.notify();
                }
            }));
        } else {
            row = row
                .on_click(cx.listener({
                    let entry_path = entry_path.clone();
                    move |this, _, _, cx| {
                        this.preview_container_file(tab_id, entry_path.clone());
                        cx.notify();
                    }
                }))
                .child(
                    Button::new(ElementId::Name(
                        format!("container-file-download-{entry_path}").into(),
                    ))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Download)
                    .on_click(cx.listener({
                        let entry_path = entry_path.clone();
                        move |this, _, _, cx| {
                            this.download_container_file(tab_id, entry_path.clone());
                            cx.notify();
                        }
                    })),
                );
        }
        row = row.child(
            Button::new(ElementId::Name(
                format!("container-file-rename-{entry_path}").into(),
            ))
            .ghost()
            .xsmall()
            .icon(IconName::Pen)
            .on_click(cx.listener({
                let entry_path = entry_path.clone();
                let entry_name = entry_name.clone();
                move |this, _, window, cx| {
                    let input =
                        cx.new(|cx| InputState::new(window, cx).default_value(entry_name.clone()));
                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
                        workspace.files.rename_from = Some(entry_path.clone());
                        workspace.files.rename_input = Some(input);
                    }
                    cx.notify();
                }
            })),
        );
        row = row.child(
            Button::new(ElementId::Name(
                format!("container-file-delete-{entry_path}").into(),
            ))
            .ghost()
            .xsmall()
            .icon(IconName::Trash)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(workspace) = this.container_workspace_mut(tab_id) {
                    workspace.confirm = Some(ContainerConfirm::FilePath {
                        path: entry_path.clone(),
                        recursive: is_dir,
                    });
                }
                cx.notify();
            })),
        );
        list = list.child(row);
    }
    if entries.is_empty() && !loading {
        list = list.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.files.empty").to_string()),
        );
    }

    let mut body = v_flex().size_full().overflow_hidden().child(
        v_flex()
            .gap_1()
            .p_2()
            .flex_none()
            .child(crumbs)
            .children(mkdir_bar)
            .children(rename_bar)
            .child(
                div()
                    .id("container-files-scroll")
                    .max_h(px(280.0))
                    .overflow_y_scroll()
                    .child(list),
            ),
    );
    if let Some((path, content, truncated)) = preview {
        let mut header = h_flex().gap_2().items_center().justify_between().child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(path.clone()),
        );
        if *truncated {
            header = header.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.files.truncated").to_string()),
            );
        }
        body = body.child(
            v_flex()
                .flex_1()
                .min_h_0()
                .p_2()
                .gap_1()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Top, p.divider_width)
                .child(header)
                .child(
                    div()
                        .id("container-preview-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p_2()
                        .rounded(p.radius.md)
                        .bg(p.muted.opacity(0.3))
                        .font_family(p.mono_font_family.clone())
                        .text_size(p.mono_font_size)
                        .child(content.clone()),
                ),
        );
    }
    body.into_any_element()
}

fn kv(p: &Palette, key: impl Into<String>, value: impl Into<String>) -> AnyElement {
    let key = key.into();
    let value = value.into();
    let value = if value.trim().is_empty() {
        "—".to_string()
    } else {
        value
    };
    h_flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(key),
        )
        .child(div().flex_1().min_w_0().text_sm().child(value))
        .into_any_element()
}

fn scroll_body(content: impl IntoElement, _p: &Palette) -> Div {
    v_flex().child(
        div()
            .id("container-detail-scroll")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .child(content),
    )
}

fn centered_hint(message: String, p: &Palette) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(p.muted_foreground)
        .child(message)
        .into_any_element()
}

fn parent_path(path: &str) -> String {
    if path == "/" || path.is_empty() {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

fn format_unix(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn short(id: &str) -> String {
    transom_core::short_id(id)
}
