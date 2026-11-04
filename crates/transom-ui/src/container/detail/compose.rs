//! 编排项目详情、服务拓扑看板与在线 YAML 编写。
//!
//! 设计原理（Why）：
//! - 拓扑看板（Overview）：按 Compose 服务成员聚合为可视化卡片网格，展示微服务间状态、
//!   镜像、暴露端口与网络连接，并支持直接联动下钻到单个容器或执行启停；
//! - 在线编写（Yaml）：依托内置 `Textarea` 提供 `docker-compose.yml` 的就地浏览与编辑，
//!   支持就地保存与一键 `docker compose up` 重新部署；
//! - 多路聚合日志（Logs）：由 `render_logs` 统一承接，自动带有各容器色块前缀。

use super::*;
use gpui_kit::component::Disableable;
use gpui_kit::component::input::{Textarea, TextareaState};
use std::path::PathBuf;
use transom_core::{
    ComposeAction, ComposeProject, ContainerLifecycleAction, ContainerRequestId, ContainerSummary,
};

/// 提取 Compose 容器的服务名（优先从镜像/名称推导简洁名）。
fn service_name_of(container: &ContainerSummary, project_name: &str) -> String {
    let raw = container.name.trim_start_matches('/');
    let prefix = format!("{project_name}_");
    let prefix_dash = format!("{project_name}-");
    if let Some(stripped) = raw.strip_prefix(&prefix).or_else(|| raw.strip_prefix(&prefix_dash)) {
        // 去除末尾序号，如 web_1 -> web, web-1 -> web
        if let Some((base, _)) = stripped.rsplit_once('_') {
            base.to_string()
        } else if let Some((base, _)) = stripped.rsplit_once('-') {
            base.to_string()
        } else {
            stripped.to_string()
        }
    } else {
        raw.to_string()
    }
}

pub(crate) fn render_compose_detail(
    tab_id: TabId,
    project: Option<&ComposeProject>,
    containers: &[ContainerSummary],
    detail_tab: ContainerDetailTab,
    compose_yaml_editor: Option<&(String, PathBuf, Entity<TextareaState>, bool)>,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(project) = project else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };

    match detail_tab {
        ContainerDetailTab::Yaml => {
            render_compose_yaml_editor(tab_id, project, compose_yaml_editor, p, window, cx)
        }
        _ => render_compose_overview_and_topology(tab_id, project, containers, p, cx),
    }
}

/// 渲染在线 YAML 编辑器。
fn render_compose_yaml_editor(
    tab_id: TabId,
    project: &ComposeProject,
    compose_yaml_editor: Option<&(String, PathBuf, Entity<TextareaState>, bool)>,
    p: &Palette,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some((_, path, editor, saving)) = compose_yaml_editor else {
        return centered_hint(
            rust_i18n::t!("container.compose.loading_yaml").to_string(),
            p,
        );
    };

    let display_path = path.display().to_string();
    let is_saving = *saving;
    let project_name = project.name.clone();
    let project_name_up = project.name.clone();

    let header = h_flex()
        .px_3()
        .py_2()
        .items_center()
        .justify_between()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.compose.yaml_file").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.muted.opacity(0.3))
                        .text_color(p.foreground)
                        .child(display_path),
                ),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Button::new("compose-yaml-save")
                        .primary()
                        .xsmall()
                        .icon(IconName::Save)
                        .label(rust_i18n::t!("common.save").to_string())
                        .disabled(is_saving)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save_compose_yaml(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("compose-yaml-deploy")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Play)
                        .label(rust_i18n::t!("container.compose.up_deploy").to_string())
                        .disabled(is_saving)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_compose(tab_id, project_name_up.clone(), ComposeAction::Up, cx);
                        })),
                )
                .child(
                    Button::new("compose-yaml-reload")
                        .ghost()
                        .xsmall()
                        .icon(IconName::RotateCw)
                        .tooltip(rust_i18n::t!("common.refresh").to_string())
                        .disabled(is_saving)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                                workspace.compose_yaml_editor = None;
                            }
                            this.ensure_compose_yaml_loaded(tab_id, &project_name, window, cx);
                        })),
                ),
        );

    v_flex()
        .size_full()
        .overflow_hidden()
        .child(header)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .p_2()
                .child(
                    div()
                        .size_full()
                        .rounded(p.radius.md)
                        .bg(p.muted.opacity(0.12))
                        .p_2()
                        .child(Textarea::new(editor).size_full()),
                ),
        )
        .into_any_element()
}

/// 渲染概览与服务拓扑看板。
fn render_compose_overview_and_topology(
    tab_id: TabId,
    project: &ComposeProject,
    containers: &[ContainerSummary],
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let members: Vec<&ContainerSummary> = containers
        .iter()
        .filter(|item| item.compose_project.as_deref() == Some(project.name.as_str()))
        .collect();

    let project_name = project.name.clone();
    let project_name_up = project.name.clone();
    let project_name_down = project.name.clone();
    let project_name_restart = project.name.clone();

    // 顶部控制与状态栏
    let action_bar = h_flex()
        .gap_2()
        .items_center()
        .justify_between()
        .p_3()
        .rounded(p.radius.lg)
        .bg(p.muted.opacity(0.15))
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .child(project.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .px_2()
                        .py_0p5()
                        .rounded(p.radius.full)
                        .bg(if project.running == project.total && project.total > 0 {
                            p.success.opacity(0.2)
                        } else if project.running > 0 {
                            p.warning.opacity(0.2)
                        } else {
                            p.muted.opacity(0.4)
                        })
                        .text_color(if project.running == project.total && project.total > 0 {
                            p.success
                        } else if project.running > 0 {
                            p.warning
                        } else {
                            p.muted_foreground
                        })
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(project.state_label()),
                ),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Button::new("compose-up-btn")
                        .primary()
                        .xsmall()
                        .icon(IconName::Play)
                        .label(rust_i18n::t!("container.action.start").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_compose(tab_id, project_name_up.clone(), ComposeAction::Up, cx);
                        })),
                )
                .child(
                    Button::new("compose-restart-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::RotateCw)
                        .label(rust_i18n::t!("container.action.restart").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_compose(tab_id, project_name_restart.clone(), ComposeAction::Up, cx);
                        })),
                )
                .child(
                    Button::new("compose-down-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Square)
                        .label(rust_i18n::t!("container.action.stop").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_compose(tab_id, project_name_down.clone(), ComposeAction::Down, cx);
                        })),
                )
                .child(
                    Button::new("compose-yaml-tab-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::FilePen)
                        .label(rust_i18n::t!("container.detail.yaml").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(w) = this.state.container_workspace_mut(tab_id) {
                                w.detail_tab = ContainerDetailTab::Yaml;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("compose-logs-tab-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ScrollText)
                        .label(rust_i18n::t!("container.detail.logs").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(w) = this.state.container_workspace_mut(tab_id) {
                                w.detail_tab = ContainerDetailTab::Logs;
                            }
                            this.ensure_container_logs(tab_id);
                            cx.notify();
                        })),
                ),
        );

    // 服务拓扑卡片看板
    let mut topology_section = v_flex().gap_2();
    topology_section = topology_section.child(
        h_flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.compose.topology_title").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.compose.service_count", count = members.len()).to_string()),
            ),
    );

    if members.is_empty() {
        topology_section = topology_section.child(
            div()
                .p_4()
                .rounded(p.radius.md)
                .bg(p.muted.opacity(0.1))
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.compose.no_containers").to_string()),
        );
    } else {
        let mut cards_grid = v_flex().gap_2();
        for member in &members {
            let service_name = service_name_of(member, &project_name);
            let is_running = member.state.is_running();
            let container_id = member.id.clone();
            let container_name = member.name.clone();
            let member_id_select = member.id.clone();
            let member_id_toggle = member.id.clone();

            let card = h_flex()
                .p_2p5()
                .gap_3()
                .items_center()
                .justify_between()
                .rounded(p.radius.md)
                .bg(p.list_background)
                .border_color(p.divider(p.border))
                .border_1()
                .child(
                    h_flex()
                        .gap_2p5()
                        .items_center()
                        .flex_1()
                        .min_w_0()
                        // 状态指示灯
                        .child(
                            div()
                                .size_2p5()
                                .rounded_full()
                                .bg(if is_running { p.success } else { p.muted_foreground }),
                        )
                        .child(
                            v_flex()
                                .gap_0p5()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::BOLD)
                                                .child(service_name),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(p.muted_foreground)
                                                .child(container_name),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(div().child(member.image.clone()))
                                        .child(div().child("·"))
                                        .child(div().child(member.status.clone())),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new(ElementId::Name(format!("toggle-{container_id}").into()))
                                .ghost()
                                .xsmall()
                                .icon(if is_running {
                                    IconName::Square
                                } else {
                                    IconName::Play
                                })
                                .tooltip(if is_running {
                                    rust_i18n::t!("container.action.stop").to_string()
                                } else {
                                    rust_i18n::t!("container.action.start").to_string()
                                })
                                .on_click(cx.listener(move |this, _, _, _cx| {
                                    let action = if is_running {
                                        ContainerLifecycleAction::Stop
                                    } else {
                                        ContainerLifecycleAction::Start
                                    };
                                    let req = ContainerRequestId::new();
                                    this.send_tracked(
                                        tab_id,
                                        req,
                                        transom_core::ContainerCommand::Lifecycle {
                                            tab_id,
                                            request_id: req,
                                            container: member_id_toggle.clone(),
                                            action,
                                        },
                                    );
                                })),
                        )
                        .child(
                            Button::new(ElementId::Name(format!("inspect-{container_id}").into()))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Info)
                                .tooltip(rust_i18n::t!("container.detail.overview").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_container_item(tab_id, member_id_select.clone());
                                    if let Some(w) = this.state.container_workspace_mut(tab_id) {
                                        w.section = ContainerSection::Containers;
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            cards_grid = cards_grid.child(card);
        }
        topology_section = topology_section.child(cards_grid);
    }

    // 基础路径信息
    let mut meta_section = v_flex().gap_1p5().pt_2();
    meta_section = meta_section
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

    let content = v_flex()
        .gap_3()
        .p_3()
        .child(action_bar)
        .child(topology_section)
        .child(meta_section);

    scroll_body(content, p).into_any_element()
}
