//! Compose 微服务依赖拓扑连线与分层看板视图。
//!
//! 设计原理（Why）：
//! - 根据 `compose_graph` 计算的 DAG 依赖图与拓扑分层，以层级看板呈现微服务群；
//! - 采用拓扑导轨连线（Tier Connector Rails）与服务引脚（Dependency Pins），清晰表达服务流向；
//! - 交互上支持鼠标悬停高亮（Hover Highlighting）：悬停任一微服务卡片时，该服务卡片亮起，
//!   其直接上游依赖（Upstream）与下游消费端（Downstream）同步点亮并标明流向，其余无关卡片自动淡化。

use super::compose_graph::*;
use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::prelude::FluentBuilder;
use std::collections::HashMap;
use transom_core::{ContainerLifecycleAction, ContainerRequestId};

/// 获取层级语义标题。
fn tier_label(tier_idx: usize, total_tiers: usize) -> String {
    if total_tiers == 1 {
        rust_i18n::t!("container.compose.topology_tier_app").to_string()
    } else if tier_idx == 0 {
        rust_i18n::t!("container.compose.topology_tier_infra").to_string()
    } else if tier_idx == total_tiers - 1 {
        rust_i18n::t!("container.compose.topology_tier_gateway").to_string()
    } else {
        rust_i18n::t!(
            "container.compose.topology_tier_service",
            tier = tier_idx + 1
        )
        .to_string()
    }
}

/// 渲染微服务拓扑分层网格看板。
pub(crate) fn render_compose_topology_board(
    view: &Entity<ContainerWorkspace>,
    project: &ComposeProject,
    members: &[&ContainerSummary],
    deps: &HashMap<String, Vec<String>>,
    hovered_service: Option<&str>,
    p: &Palette,
) -> AnyElement {
    if members.is_empty() {
        return div()
            .p_4()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.1))
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.compose.no_containers").to_string())
            .into_any_element();
    }

    let rev_deps = build_dependents_map(deps);

    // 收集所有服务名并建立成员映射
    let mut member_by_service: HashMap<String, &ContainerSummary> = HashMap::new();
    let mut all_service_names: Vec<String> = Vec::new();

    for member in members {
        let name = super::compose::service_name_of(member, &project.name);
        if !member_by_service.contains_key(&name) {
            all_service_names.push(name.clone());
        }
        member_by_service.insert(name, member);
    }

    let tiers = calculate_topology_tiers(&all_service_names, deps);
    let total_tiers = tiers.len();

    let mut board = v_flex().gap_3().w_full();

    for (tier_idx, tier_services) in tiers.iter().enumerate() {
        let is_last = tier_idx + 1 == total_tiers;
        let tier_name = tier_label(tier_idx, total_tiers);

        let mut tier_block = v_flex().gap_2().w_full();

        // 层级头部指示栏
        let tier_header = h_flex()
            .gap_2()
            .items_center()
            .child(div().size_2().rounded_full().bg(if tier_idx == 0 {
                p.primary
            } else if is_last {
                p.warning
            } else {
                p.muted_foreground
            }))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.muted_foreground)
                    .child(tier_name),
            )
            .child(
                div().text_xs().text_color(p.text.muted).child(
                    rust_i18n::t!(
                        "container.compose.service_count",
                        count = tier_services.len()
                    )
                    .to_string(),
                ),
            );

        tier_block = tier_block.child(tier_header);

        // 卡片网格
        let mut cards_container = v_flex().gap_2().w_full();
        for svc_name in tier_services {
            if let Some(member) = member_by_service.get(svc_name) {
                let card = render_topology_card(
                    view,
                    member,
                    TopologyContext {
                        service_name: svc_name,
                        deps,
                        rev_deps: &rev_deps,
                        hovered_service,
                    },
                    p,
                );
                cards_container = cards_container.child(card);
            }
        }
        tier_block = tier_block.child(cards_container);

        // 层级之间的流动导轨连接线
        if !is_last {
            tier_block = tier_block.child(render_tier_connector_line(p));
        }

        board = board.child(tier_block);
    }

    board.into_any_element()
}

/// 渲染层与层之间的拓扑连接导轨线与箭头。
fn render_tier_connector_line(p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_center()
        .py_1()
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_0p5()
                .rounded(p.radius.full)
                .bg(p.muted.opacity(0.06))
                .border_1()
                .border_color(p.border.opacity(0.2))
                .child(div().w(px(24.0)).h(px(1.0)).bg(p.stroke.strong))
                .child(
                    Icon::new(IconName::ArrowDown)
                        .with_size(px(11.0))
                        .text_color(p.muted_foreground),
                )
                .child(div().w(px(24.0)).h(px(1.0)).bg(p.stroke.strong)),
        )
        .into_any_element()
}

/// 拓扑卡片依赖上下文。
struct TopologyContext<'a> {
    service_name: &'a str,
    deps: &'a HashMap<String, Vec<String>>,
    rev_deps: &'a HashMap<String, Vec<String>>,
    hovered_service: Option<&'a str>,
}

/// 渲染单个微服务拓扑卡片。
fn render_topology_card(
    view: &Entity<ContainerWorkspace>,
    member: &ContainerSummary,
    context: TopologyContext<'_>,
    p: &Palette,
) -> AnyElement {
    let TopologyContext {
        service_name,
        deps,
        rev_deps,
        hovered_service,
    } = context;
    let highlight = calculate_highlight_state(service_name, hovered_service, deps, rev_deps);

    let is_running = member.state.is_running();
    let container_id = member.id.clone();
    let container_name = member.name.clone();
    let member_id_select = member.id.clone();
    let member_id_toggle = member.id.clone();
    let current_svc_name = service_name.to_string();

    let upstream_deps = deps.get(service_name).cloned().unwrap_or_default();
    let downstream_dependents = rev_deps.get(service_name).cloned().unwrap_or_default();

    let (border_color, border_width, opacity_val, tag_badge) = match highlight {
        TopologyHighlightState::Normal => (p.divider(p.border), px(1.0), 1.0, None),
        TopologyHighlightState::Focused => (
            p.primary,
            px(1.5),
            1.0,
            Some((
                rust_i18n::t!("container.compose.topology_title").to_string(),
                p.primary,
            )),
        ),
        TopologyHighlightState::Upstream => (
            p.success,
            px(1.5),
            1.0,
            Some((
                rust_i18n::t!("container.compose.topology_upstream").to_string(),
                p.success,
            )),
        ),
        TopologyHighlightState::Downstream => (
            p.warning,
            px(1.5),
            1.0,
            Some((
                rust_i18n::t!("container.compose.topology_downstream").to_string(),
                p.warning,
            )),
        ),
        TopologyHighlightState::Dimmed => (p.divider(p.border), px(1.0), 0.38, None),
    };

    let hover_svc = current_svc_name.clone();
    let hover_view = view.clone();

    let mut card = v_flex()
        .id(ElementId::Name(
            format!("compose-card-{service_name}").into(),
        ))
        .w_full()
        .p_2p5()
        .gap_2()
        .rounded(p.radius.md)
        .bg(p.list_background)
        .border_color(border_color)
        .border(border_width)
        .opacity(opacity_val)
        .on_hover(move |hovered: &bool, _, cx| {
            hover_view.update(cx, |workspace, cx| {
                if *hovered {
                    workspace.state.compose_hovered_service = Some(hover_svc.clone());
                } else if workspace.state.compose_hovered_service.as_deref() == Some(&hover_svc) {
                    workspace.state.compose_hovered_service = None;
                }
                cx.notify();
            });
        });

    let mut main_row = h_flex().items_center().justify_between().w_full();

    let left_info = h_flex()
        .gap_2p5()
        .items_center()
        .flex_1()
        .min_w_0()
        .child(div().size_2p5().rounded_full().bg(if is_running {
            p.success
        } else {
            p.muted_foreground
        }))
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
                                .child(service_name.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(container_name),
                        )
                        .when_some(tag_badge, |this, (label, color)| {
                            this.child(
                                div()
                                    .text_xs()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(p.radius.sm)
                                    .bg(color.opacity(0.12))
                                    .text_color(color)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(label),
                            )
                        }),
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
        );

    let toggle_view = view.clone();
    let inspect_view = view.clone();
    let actions = h_flex()
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
                .accessibility_label(if is_running {
                    rust_i18n::t!("container.action.stop").to_string()
                } else {
                    rust_i18n::t!("container.action.start").to_string()
                })
                .tooltip(if is_running {
                    rust_i18n::t!("container.action.stop").to_string()
                } else {
                    rust_i18n::t!("container.action.start").to_string()
                })
                .on_click(move |_, _, cx| {
                    let action = if is_running {
                        ContainerLifecycleAction::Stop
                    } else {
                        ContainerLifecycleAction::Start
                    };
                    toggle_view.update(cx, |workspace, _| {
                        let request_id = ContainerRequestId::new();
                        workspace.send_tracked(
                            request_id,
                            transom_core::ContainerCommand::Lifecycle {
                                tab_id: workspace.tab_id,
                                request_id,
                                container: member_id_toggle.clone(),
                                action,
                            },
                        );
                    });
                }),
        )
        .child(
            Button::new(ElementId::Name(format!("inspect-{container_id}").into()))
                .ghost()
                .xsmall()
                .icon(IconName::Info)
                .accessibility_label(rust_i18n::t!("container.detail.overview").to_string())
                .tooltip(rust_i18n::t!("container.detail.overview").to_string())
                .on_click(move |_, _, cx| {
                    inspect_view.update(cx, |workspace, cx| {
                        workspace.select_container_item(member_id_select.clone());
                        workspace.state.section = ContainerSection::Containers;
                        cx.notify();
                    });
                }),
        );

    main_row = main_row.child(left_info).child(actions);
    card = card.child(main_row);

    // 依赖拓扑引脚关系栏：呈现本服务的上下游连接脉络
    if !upstream_deps.is_empty() || !downstream_dependents.is_empty() {
        let mut pins_bar = h_flex()
            .gap_3()
            .items_center()
            .pt_1()
            .border_t_1()
            .border_color(p.stroke.subtle)
            .text_xs();

        // 依赖的上游
        if !upstream_deps.is_empty() {
            let mut up_row = h_flex().gap_1p5().items_center();
            up_row = up_row.child(div().text_color(p.muted_foreground).child(format!(
                "↳ {}:",
                rust_i18n::t!("container.compose.topology_depends_on")
            )));
            for dep in &upstream_deps {
                let is_dep_focused = hovered_service == Some(dep.as_str());
                up_row = up_row.child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(if is_dep_focused {
                            p.primary.opacity(0.15)
                        } else {
                            p.surface.hover
                        })
                        .text_color(if is_dep_focused {
                            p.primary
                        } else {
                            p.foreground
                        })
                        .font_weight(if is_dep_focused {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(dep.clone()),
                );
            }
            pins_bar = pins_bar.child(up_row);
        }

        // 供给的下游
        if !downstream_dependents.is_empty() {
            let mut down_row = h_flex().gap_1p5().items_center();
            down_row = down_row.child(div().text_color(p.muted_foreground).child(format!(
                "↱ {}:",
                rust_i18n::t!("container.compose.topology_dependents")
            )));
            for down in &downstream_dependents {
                let is_down_focused = hovered_service == Some(down.as_str());
                down_row = down_row.child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(if is_down_focused {
                            p.warning.opacity(0.15)
                        } else {
                            p.surface.hover
                        })
                        .text_color(if is_down_focused {
                            p.warning
                        } else {
                            p.foreground
                        })
                        .font_weight(if is_down_focused {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(down.clone()),
                );
            }
            pins_bar = pins_bar.child(down_row);
        }

        card = card.child(pins_bar);
    }

    card.into_any_element()
}
