//! 构建弹窗的渲染。

use super::super::*;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染镜像构建弹窗主体。
///
/// 设计原理（Why）：构建参数、日志、错误与成功标记都归工作区实体，渲染只读
/// 快照；浏览上下文、切换缓存、取消与提交都直接 `update` 实体。
pub(crate) fn render_container_build_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(build) = state.modals.build_dialog.as_ref() else {
        return div().into_any_element();
    };
    let tag_display = build.tag.read(cx).value().to_string();

    v_flex()
        .w_full()
        .gap_3()
        .child(render_build_context_row(view, &build.context_path, p))
        .child(render_build_two_col_inputs(
            "container.build.dockerfile",
            &build.dockerfile,
            "container.build.tag",
            &build.tag,
            p,
        ))
        .child(render_build_two_col_inputs(
            "container.build.target",
            &build.target,
            "container.build.platform",
            &build.platform,
            p,
        ))
        .child(render_build_labeled_input(
            "container.build.build_args",
            &build.build_args,
            p,
        ))
        .child(render_build_no_cache_toggle(view, build.no_cache, p))
        .when(build.building, |this| this.child(render_building_bar(p)))
        .when(!build.logs.is_empty() || build.building, |this| {
            this.child(render_build_logs(&build.logs, p))
        })
        .when(build.success, |this| {
            this.child(render_build_banner(
                AppIcon::Ui(UiIcon::CHECK),
                p.tone.success_subtle,
                p.success,
                rust_i18n::t!("container.build.success", tag = tag_display).to_string(),
            ))
        })
        .when_some(build.error.clone(), |this, err| {
            this.child(render_build_banner(
                AppIcon::Ui(UiIcon::WARNING),
                p.tone.danger_subtle,
                p.danger,
                rust_i18n::t!("container.build.failed", error = err).to_string(),
            ))
        })
        .child(render_build_footer(view, build.building))
        .into_any_element()
}

fn render_build_labeled_input(
    label_key: &str,
    input: &Entity<gpui_kit::component::input::InputState>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .flex_1()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!(label_key).to_string()),
        )
        .child(Input::new(input).w_full())
        .into_any_element()
}

fn render_build_two_col_inputs(
    left_key: &str,
    left: &Entity<gpui_kit::component::input::InputState>,
    right_key: &str,
    right: &Entity<gpui_kit::component::input::InputState>,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .gap_3()
        .child(render_build_labeled_input(left_key, left, p))
        .child(render_build_labeled_input(right_key, right, p))
        .into_any_element()
}

fn render_build_context_row(
    view: &Entity<ContainerWorkspace>,
    context_path: &Entity<gpui_kit::component::input::InputState>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.build.context_path").to_string()),
        )
        .child(
            h_flex()
                .gap_2()
                .child(Input::new(context_path).w_full())
                .child(
                    Button::new("container-build-browse")
                        .secondary()
                        .small()
                        .label(rust_i18n::t!("container.build.browse").to_string())
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |workspace, cx| {
                                    workspace.browse_build_context_path(window, cx);
                                });
                            }
                        }),
                ),
        )
        .into_any_element()
}

fn render_build_no_cache_toggle(
    view: &Entity<ContainerWorkspace>,
    no_cache: bool,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .id("container-build-no-cache")
        .gap_2()
        .items_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.toggle_build_no_cache(cx));
            }
        })
        .child(
            AppIconElement::new(if no_cache {
                AppIcon::Ui(UiIcon::CHECK)
            } else {
                AppIcon::Ui(UiIcon::STOP)
            })
            .xsmall()
            .text_color(if no_cache {
                p.primary
            } else {
                p.muted_foreground
            }),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.foreground)
                .child(rust_i18n::t!("container.build.no_cache").to_string()),
        )
        .into_any_element()
}

fn render_building_bar(p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH))
                .xsmall()
                .text_color(p.primary),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.primary)
                .child(rust_i18n::t!("container.build.building").to_string()),
        )
        .child(
            div()
                .flex_1()
                .child(crate::widgets::progress_bar(1.0, p.primary, p)),
        )
        .into_any_element()
}

fn render_build_logs(logs: &[String], p: &Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.build.logs_title").to_string()),
        )
        .child(
            div()
                .id("container-build-logs-scroll")
                .p_2()
                .rounded(p.radius.lg)
                .bg(p.surface.ghost)
                .border_1()
                .border_color(p.border.opacity(0.18))
                .text_color(p.foreground)
                .font_family(p.mono_font_family.clone())
                .h(px(140.0))
                .overflow_y_scroll()
                .children(if logs.is_empty() {
                    vec![
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.build.waiting_logs").to_string())
                            .into_any_element(),
                    ]
                } else {
                    logs.iter()
                        .map(|line| div().text_xs().child(line.clone()).into_any_element())
                        .collect()
                }),
        )
        .into_any_element()
}

fn render_build_banner(icon: AppIcon, bg: Hsla, fg: Hsla, text: String) -> AnyElement {
    h_flex()
        .w_full()
        .p_3()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .bg(bg)
        .text_color(fg)
        .child(AppIconElement::new(icon).small())
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(text))
        .into_any_element()
}

fn render_build_footer(view: &Entity<ContainerWorkspace>, building: bool) -> AnyElement {
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("container-build-cancel")
                .ghost()
                .small()
                .label(rust_i18n::t!("common.cancel").to_string())
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| workspace.close_build_dialog(cx));
                    }
                }),
        )
        .child(
            Button::new("container-build-submit")
                .primary()
                .small()
                .disabled(building)
                .label(if building {
                    rust_i18n::t!("container.build.building").to_string()
                } else {
                    rust_i18n::t!("container.build.start_build").to_string()
                })
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| workspace.submit_build_dialog(cx));
                    }
                }),
        )
        .into_any_element()
}
