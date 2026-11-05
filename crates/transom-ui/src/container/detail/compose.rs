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
use transom_core::{ComposeAction, ComposeProject, ContainerSummary};

/// 提取 Compose 容器的服务名（优先从镜像/名称推导简洁名）。
pub(crate) fn service_name_of(container: &ContainerSummary, project_name: &str) -> String {
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

    // 顶部状态横幅：呈现项目名称、运行状态与微服务容器数量（不再与顶栏按钮重复）
    let action_bar = h_flex()
        .gap_2()
        .items_center()
        .justify_between()
        .p_3()
        .rounded(p.radius.lg)
        .bg(p.muted.opacity(0.04))
        .border_1()
        .border_color(p.border.opacity(0.3))
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
                            p.success.opacity(0.12)
                        } else if project.running > 0 {
                            p.warning.opacity(0.12)
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
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!(
                    "container.compose.service_count",
                    count = members.len()
                )),
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

    // 确保异步解析依赖
    let project_name = project.name.clone();
    let needs_load = cx
        .entity()
        .read(cx)
        .state
        .container_workspace(tab_id)
        .is_some_and(|w| w.compose_dependencies.is_empty());
    if needs_load {
        let app = cx.entity().clone();
        cx.defer(move |cx| {
            app.update(cx, |this, cx| {
                this.ensure_compose_dependencies_loaded(tab_id, &project_name, cx);
            });
        });
    }

    let (deps, hovered_service) = cx
        .entity()
        .read(cx)
        .state
        .container_workspace(tab_id)
        .map(|w| (w.compose_dependencies.clone(), w.compose_hovered_service.clone()))
        .unwrap_or_default();

    let topology_board = super::compose_topology::render_compose_topology_board(
        tab_id,
        project,
        &members,
        &deps,
        hovered_service.as_deref(),
        p,
        cx,
    );
    topology_section = topology_section.child(topology_board);

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
