//! 构建弹窗的渲染。

use super::super::*;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

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
    let Some(build) = state.build_dialog.as_ref() else {
        return div().into_any_element();
    };

    let context_path = build.context_path.clone();
    let dockerfile = build.dockerfile.clone();
    let tag = build.tag.clone();
    let target = build.target.clone();
    let platform = build.platform.clone();
    let build_args = build.build_args.clone();
    let no_cache = build.no_cache;
    let building = build.building;
    let logs = build.logs.clone();
    let error = build.error.clone();
    let success = build.success;
    let tag_display = build.tag.read(cx).value().to_string();

    v_flex()
        .w_full()
        .gap_3()
        .child(
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
                        .child(Input::new(&context_path).w_full())
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
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.build.dockerfile").to_string()),
                        )
                        .child(Input::new(&dockerfile).w_full()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.build.tag").to_string()),
                        )
                        .child(Input::new(&tag).w_full()),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.build.target").to_string()),
                        )
                        .child(Input::new(&target).w_full()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.build.platform").to_string()),
                        )
                        .child(Input::new(&platform).w_full()),
                ),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.build.build_args").to_string()),
                )
                .child(Input::new(&build_args).w_full()),
        )
        .child(
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
                    Icon::new(if no_cache {
                        IconName::Check
                    } else {
                        IconName::Square
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
                ),
        )
        .when(building, |this| {
            this.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(IconName::Loader).xsmall().text_color(p.primary))
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
                    ),
            )
        })
        .when(!logs.is_empty() || building, |this| {
            this.child(
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
                            .bg(p.muted.opacity(0.04))
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
                                        .child(
                                            rust_i18n::t!("container.build.waiting_logs")
                                                .to_string(),
                                        )
                                        .into_any_element(),
                                ]
                            } else {
                                logs.into_iter()
                                    .map(|line| div().text_xs().child(line).into_any_element())
                                    .collect()
                            }),
                    ),
            )
        })
        .when(success, |this| {
            this.child(
                h_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .items_center()
                    .rounded(p.radius.lg)
                    .bg(p.success.opacity(0.12))
                    .text_color(p.success)
                    .child(Icon::new(IconName::Check).small())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(rust_i18n::t!("container.build.success", tag = tag_display)),
                    ),
            )
        })
        .when_some(error, |this, err| {
            this.child(
                h_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .items_center()
                    .rounded(p.radius.lg)
                    .bg(p.danger.opacity(0.12))
                    .text_color(p.danger)
                    .child(Icon::new(IconName::TriangleAlert).small())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(rust_i18n::t!("container.build.failed", error = err)),
                    ),
            )
        })
        .child(
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
                ),
        )
        .into_any_element()
}
