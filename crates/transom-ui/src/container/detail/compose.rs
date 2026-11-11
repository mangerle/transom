//! 编排项目详情、服务拓扑看板与在线 YAML 编写。
//!
//! 设计原理（Why）：
//! - 拓扑看板（Overview）：按 Compose 服务成员聚合为可视化卡片网格，展示微服务间状态、
//!   镜像、暴露端口与网络连接，并支持直接联动下钻到单个容器或执行启停；
//! - 在线编写（Yaml）：依托内置 `Textarea` 提供 `docker-compose.yml` 的就地浏览与编辑，
//!   支持就地保存与一键 `docker compose up` 重新部署；
//! - 多路聚合日志（Logs）：由 `render_logs` 统一承接，自动带有各容器色块前缀。
//!
//! 渲染只持有工作区实体句柄与只读快照：保存、部署与重新加载都直接改实体状态或
//! 调用实体方法，不再依赖 `Context<AppShell>`；编辑器文本由外壳在调用前快照。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::component::Disableable;
use gpui_kit::component::input::{Textarea, TextareaState};
use std::path::PathBuf;
use transom_core::{ComposeAction, ComposeProject, ContainerSummary};

/// 提取 Compose 容器的服务名（优先从镜像/名称推导简洁名）。
pub(crate) fn service_name_of(container: &ContainerSummary, project_name: &str) -> String {
    let raw = container.name.trim_start_matches('/');
    let prefix = format!("{project_name}_");
    let prefix_dash = format!("{project_name}-");
    if let Some(stripped) = raw
        .strip_prefix(&prefix)
        .or_else(|| raw.strip_prefix(&prefix_dash))
    {
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

/// Compose 详情与拓扑渲染参数。
pub(crate) struct ComposeDetailParams<'a> {
    pub(crate) project: Option<&'a ComposeProject>,
    pub(crate) containers: &'a [ContainerSummary],
    pub(crate) detail_tab: ContainerDetailTab,
    pub(crate) compose_yaml_editor: Option<&'a (String, PathBuf, Entity<TextareaState>, bool)>,
    /// 编辑器当前文本快照：行号与语法校验在纯渲染阶段使用。
    pub(crate) compose_yaml_text: &'a str,
    pub(crate) dependencies: &'a std::collections::HashMap<String, Vec<String>>,
    pub(crate) hovered_service: Option<&'a str>,
}

pub(crate) fn render_compose_detail(
    view: &Entity<ContainerWorkspace>,
    params: ComposeDetailParams<'_>,
    p: &Palette,
) -> AnyElement {
    let Some(project) = params.project else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };

    match params.detail_tab {
        ContainerDetailTab::Yaml => render_compose_yaml_editor(
            view,
            project,
            params.compose_yaml_editor,
            params.compose_yaml_text,
            p,
        ),
        _ => render_compose_overview_and_topology(
            view,
            project,
            params.containers,
            params.dependencies,
            params.hovered_service,
            p,
        ),
    }
}

use super::compose_yaml::{render_line_numbers, render_path_breadcrumbs, validate_compose_yaml};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染在线 YAML 编辑器。
///
/// 保存直接调用实体方法；部署属于纯实体动作；重新加载清空编辑器后由实体
/// 重新读取文件，编辑器实体仍由外壳在拿到 `Window` 的渲染期创建。
fn render_compose_yaml_editor(
    view: &Entity<ContainerWorkspace>,
    project: &ComposeProject,
    compose_yaml_editor: Option<&(String, PathBuf, Entity<TextareaState>, bool)>,
    current_text: &str,
    p: &Palette,
) -> AnyElement {
    let Some((_, path, editor, saving)) = compose_yaml_editor else {
        return centered_hint(
            rust_i18n::t!("container.compose.loading_yaml").to_string(),
            p,
        );
    };

    let is_saving = *saving;
    let project_name_up = project.name.clone();

    // 行数与语法校验基于外壳传入的文本快照，渲染函数不再读取编辑器实体。
    let line_count = current_text.lines().count().max(1);
    let validation_err = validate_compose_yaml(current_text);

    let save_view = view.clone();
    let deploy_view = view.clone();
    let reload_view = view.clone();

    let header = h_flex()
        .px_3()
        .py_2()
        .items_center()
        .justify_between()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(render_path_breadcrumbs(&project.name, path, p))
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Button::new("compose-yaml-save")
                        .primary()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::SAVE)).small())
                        .label(rust_i18n::t!("common.save").to_string())
                        .disabled(is_saving)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |workspace, cx| {
                                workspace.save_compose_yaml(cx);
                            });
                        }),
                )
                .child(
                    Button::new("compose-yaml-deploy")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                        .label(rust_i18n::t!("container.compose.up_deploy").to_string())
                        .disabled(is_saving)
                        .on_click(move |_, _, cx| {
                            deploy_view.update(cx, |workspace, cx| {
                                workspace.run_compose(
                                    project_name_up.clone(),
                                    ComposeAction::Up,
                                    cx,
                                );
                            });
                        }),
                )
                .child(
                    Button::new("compose-yaml-reload")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                        .accessibility_label(rust_i18n::t!("common.refresh").to_string())
                        .tooltip(rust_i18n::t!("common.refresh").to_string())
                        .disabled(is_saving)
                        .on_click(move |_, _, cx| {
                            reload_view.update(cx, |workspace, cx| {
                                workspace.state.compose_yaml_editor = None;
                                workspace.ensure_compose_yaml_loaded(cx);
                                cx.notify();
                            });
                        }),
                ),
        );

    // 底部语法校验状态横幅
    let status_banner = match &validation_err {
        Some(err) => h_flex()
            .w_full()
            .px_3()
            .py_1()
            .gap_1p5()
            .items_center()
            .bg(p.danger.opacity(0.08))
            .text_color(p.danger)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::WARNING)).xsmall())
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(rust_i18n::t!(
                        "container.compose.yaml_syntax_error",
                        line = err.line,
                        msg = err.message.clone()
                    )),
            )
            .into_any_element(),
        None => h_flex()
            .w_full()
            .px_3()
            .py_1()
            .gap_1p5()
            .items_center()
            .bg(p.success.opacity(0.04))
            .text_color(p.success)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).xsmall())
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(rust_i18n::t!("container.compose.yaml_syntax_valid").to_string()),
            )
            .into_any_element(),
    };

    let editor_area = h_flex()
        .size_full()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.06))
        .overflow_hidden()
        .child(render_line_numbers(
            line_count,
            validation_err.as_ref().map(|e| e.line),
            p,
        ))
        .child(
            div()
                .flex_1()
                .size_full()
                .p_2()
                .child(Textarea::new(editor).size_full()),
        );

    v_flex()
        .size_full()
        .overflow_hidden()
        .child(header)
        .child(div().flex_1().min_h_0().p_2().child(editor_area))
        .child(status_banner)
        .into_any_element()
}

/// 渲染概览与服务拓扑看板。
fn render_compose_overview_and_topology(
    view: &Entity<ContainerWorkspace>,
    project: &ComposeProject,
    containers: &[ContainerSummary],
    dependencies: &std::collections::HashMap<String, Vec<String>>,
    hovered_service: Option<&str>,
    p: &Palette,
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
        .bg(p.surface.ghost)
        .border_1()
        .border_color(p.stroke.muted)
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
                            p.tone.success_subtle
                        } else if project.running > 0 {
                            p.tone.warning_subtle
                        } else {
                            p.surface.pressed
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
            .child(div().text_xs().text_color(p.muted_foreground).child(
                rust_i18n::t!("container.compose.service_count", count = members.len()).to_string(),
            )),
    );

    let topology_board = super::compose_topology::render_compose_topology_board(
        view,
        project,
        &members,
        dependencies,
        hovered_service,
        p,
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
