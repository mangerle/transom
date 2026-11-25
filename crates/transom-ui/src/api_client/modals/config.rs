//! 环境变量与全局配置模态弹窗：左侧导航（全局/环境管理）与右侧编辑区。

use crate::api_client::ApiConfigModalSection;
use crate::api_client::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use crate::widgets::{IconButton, IconButtonSize, action_button_with_icon_tint};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

/// 渲染环境变量、全局变量、全局参数与 Cookie 管理器弹窗。
pub(in crate::api_client) fn render_config_modal(
    params: &ApiWorkspaceParams<'_>,
    section: ApiConfigModalSection,
    p: &Palette,
) -> AnyElement {
    let backdrop_view = params.view.clone();
    let card = h_flex()
        .id("api-config-modal-card")
        .w(rems(58.0))
        .h(rems(38.0))
        .max_w(relative(0.9))
        .max_h(relative(0.88))
        .bg(p.background)
        .rounded(p.radius.lg)
        .border_1()
        .border_color(p.divider(p.border))
        .shadow_lg()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(render_sidebar(params, section, p))
        .child(render_content(params, section, p));

    div()
        .id("api-config-modal-backdrop")
        .absolute()
        .inset_0()
        .bg(gpui::black().opacity(0.45))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            backdrop_view.update(cx, |this, cx| this.close_config_modal(cx));
        })
        .child(card)
        .into_any_element()
}

fn render_sidebar(
    params: &ApiWorkspaceParams<'_>,
    section: ApiConfigModalSection,
    p: &Palette,
) -> AnyElement {
    let add_view = params.view.clone();
    let mut env_rows = v_flex().w_full().gap(rems(space::XXS));
    let can_delete = params.project.environments.len() > 1;
    for environment in &params.project.environments {
        let env_id = environment.id;
        let selected = section == ApiConfigModalSection::Environment(env_id);
        let select_view = params.view.clone();
        let delete_view = params.view.clone();
        env_rows = env_rows.child(
            h_flex()
                .id(ElementId::Name(format!("api-config-env-{env_id}").into()))
                .w_full()
                .h(rems(size::FILE_ROW_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::XS))
                .items_center()
                .rounded(p.radius.md)
                .cursor_pointer()
                .when(selected, |row| row.bg(p.surface.control_selected))
                .when(!selected, |row| {
                    row.hover(|style| style.bg(p.sidebar_accent.opacity(0.6)))
                })
                .on_click(move |_, window, cx| {
                    select_view.update(cx, |this, cx| {
                        this.select_config_modal_section(
                            ApiConfigModalSection::Environment(env_id),
                            window,
                            cx,
                        );
                    });
                })
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::DEPENDENCIES))
                        .xsmall()
                        .text_color(if selected {
                            p.primary
                        } else {
                            p.muted_foreground
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rems(typography::BODY))
                        .text_color(if selected { p.primary } else { p.foreground })
                        .child(environment.name.clone()),
                )
                .when(can_delete, |row| {
                    row.child(
                        IconButton::new(
                            ElementId::Name(format!("api-config-env-del-{env_id}").into()),
                            AppIcon::Ui(UiIcon::DELETE),
                            rust_i18n::t!("api.request.delete_environment").to_string(),
                        )
                        .size(IconButtonSize::Compact)
                        .render()
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            delete_view.update(cx, |this, cx| {
                                this.delete_modal_environment(env_id, window, cx)
                            });
                        }),
                    )
                }),
        );
    }

    v_flex()
        .w(rems(14.5))
        .flex_none()
        .h_full()
        .bg(p.sidebar)
        .border_r_1()
        .border_color(p.divider(p.border))
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .child(section_label(
            rust_i18n::t!("api.request.global_section").to_string(),
            p,
        ))
        .child(nav_item(
            "api-config-nav-cookies",
            AppIcon::Custom(CustomIcon::COOKIE),
            rust_i18n::t!("api.request.cookie_manager").to_string(),
            section == ApiConfigModalSection::Cookies,
            ApiConfigModalSection::Cookies,
            params.view,
            p,
        ))
        .child(nav_item(
            "api-config-nav-global-params",
            AppIcon::Ui(UiIcon::DATAFLOW),
            rust_i18n::t!("api.request.global_parameters").to_string(),
            section == ApiConfigModalSection::GlobalParameters,
            ApiConfigModalSection::GlobalParameters,
            params.view,
            p,
        ))
        .child(nav_item(
            "api-config-nav-global-vars",
            AppIcon::Ui(UiIcon::LANGUAGE),
            rust_i18n::t!("api.request.global_variables").to_string(),
            section == ApiConfigModalSection::GlobalVariables,
            ApiConfigModalSection::GlobalVariables,
            params.view,
            p,
        ))
        .child(div().pt(rems(space::SM)).child(section_label(
            rust_i18n::t!("api.request.environment_management").to_string(),
            p,
        )))
        .child(
            div()
                .id("api-config-env-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(env_rows),
        )
        .child(
            div().pt(rems(space::XS)).child(
                Button::new("api-config-new-env")
                    .outline()
                    .small()
                    .w_full()
                    .label(format!(
                        "+ {}",
                        rust_i18n::t!("api.request.new_environment")
                    ))
                    .on_click(move |_, window, cx| {
                        add_view.update(cx, |this, cx| this.create_modal_environment(window, cx));
                    }),
            ),
        )
        .into_any_element()
}

fn section_label(text: String, p: &Palette) -> AnyElement {
    div()
        .px(rems(space::SM))
        .py(rems(space::XXS))
        .text_size(rems(typography::CAPTION))
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}

fn nav_item(
    id: &'static str,
    icon: AppIcon,
    label: String,
    selected: bool,
    target: ApiConfigModalSection,
    view: &Entity<crate::api_client::ApiWorkspace>,
    p: &Palette,
) -> AnyElement {
    let view = view.clone();
    h_flex()
        .id(id)
        .w_full()
        .h(rems(size::FILE_ROW_HEIGHT))
        .px(rems(space::SM))
        .gap(rems(space::XS))
        .items_center()
        .rounded(p.radius.md)
        .cursor_pointer()
        .when(selected, |row| row.bg(p.surface.control_selected))
        .when(!selected, |row| {
            row.hover(|style| style.bg(p.sidebar_accent.opacity(0.6)))
        })
        .on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
                this.select_config_modal_section(target, window, cx)
            });
        })
        .child(AppIconElement::new(icon).xsmall().text_color(if selected {
            p.primary
        } else {
            p.muted_foreground
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(typography::BODY))
                .text_color(if selected { p.primary } else { p.foreground })
                .child(label),
        )
        .into_any_element()
}

fn render_content(
    params: &ApiWorkspaceParams<'_>,
    section: ApiConfigModalSection,
    p: &Palette,
) -> AnyElement {
    let close_view = params.view.clone();
    let save_close_view = params.view.clone();
    let save_view = params.view.clone();
    let (title, body) = match section {
        ApiConfigModalSection::Cookies => (
            rust_i18n::t!("api.request.cookie_manager").to_string(),
            params.editors.cookies.clone().into_any_element(),
        ),
        ApiConfigModalSection::GlobalParameters => (
            rust_i18n::t!("api.request.global_parameters").to_string(),
            crate::api_client::workspace::global_params_view::render_global_parameters(params, p),
        ),
        ApiConfigModalSection::GlobalVariables => (
            rust_i18n::t!("api.request.global_variables").to_string(),
            params.editors.global_variables.clone().into_any_element(),
        ),
        ApiConfigModalSection::Environment(_) => {
            let env_body = v_flex()
                .size_full()
                .min_h_0()
                .gap(rems(space::SM))
                .child(
                    v_flex()
                        .gap(rems(space::XXS))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("api.request.environment_name").to_string()),
                        )
                        .child(Input::new(&params.editors.environment_name).w_full()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap(rems(space::XXS))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.muted_foreground)
                                .child(
                                    rust_i18n::t!("api.request.environment_variables").to_string(),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .rounded(p.radius.md)
                                .border_1()
                                .border_color(p.divider(p.border))
                                .overflow_hidden()
                                .child(params.editors.environment.clone()),
                        ),
                )
                .into_any_element();
            (
                rust_i18n::t!("api.request.environment_management").to_string(),
                env_body,
            )
        }
    };

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.background)
        .child(
            h_flex()
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .px(rems(space::MD))
                .items_center()
                .border_b_1()
                .border_color(p.divider(p.border))
                .child(
                    div()
                        .flex_1()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(title),
                )
                .child(
                    IconButton::new(
                        "api-config-modal-close",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("common.close").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        close_view.update(cx, |this, cx| this.close_config_modal(cx));
                    }),
                ),
        )
        .child(div().flex_1().min_h_0().p(rems(space::MD)).child(body))
        .child(
            h_flex()
                .px(rems(space::MD))
                .py(rems(space::SM))
                .gap(rems(space::SM))
                .justify_end()
                .border_t_1()
                .border_color(p.divider(p.border))
                .child(
                    Button::new("api-config-modal-save-close")
                        .outline()
                        .small()
                        .label(rust_i18n::t!("api.request.save_and_close").to_string())
                        .on_click(move |_, _, cx| {
                            save_close_view.update(cx, |this, cx| {
                                this.save_project(cx);
                                this.close_config_modal(cx);
                            });
                        }),
                )
                .child(
                    action_button_with_icon_tint(
                        "api-config-modal-save",
                        AppIcon::Ui(UiIcon::CHECK),
                        rust_i18n::t!("api.request.save").to_string(),
                        Some(gpui::white()),
                    )
                    .primary()
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |this, cx| this.save_project(cx));
                    }),
                ),
        )
        .into_any_element()
}
