//! 偏好设置中容器镜像仓库列表渲染。
//!
//! 设计原理（Why）：
//! - 将镜像仓库卡片列表自绘逻辑独立拆分，解耦纯视图呈现与状态提交；
//! - 遵循统一的设计令牌，展示官方徽章、默认源指示胶囊与认证摘要。

use super::super::SettingsView;
use crate::app::app_settings;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl SettingsView {
    /// 渲染仓库列表界面。
    pub(super) fn render_registries_list(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let registries = app_settings(cx).container_registries.clone();
        let top_bar = registry_top_bar(p, cx);
        let mut list = v_flex().w_full().gap_2p5();
        for reg in &registries {
            list = list.child(registry_card(reg, p, cx));
        }
        v_flex()
            .w_full()
            .gap_3()
            .child(top_bar)
            .child(list)
            .into_any_element()
    }
}

fn registry_top_bar(p: &Palette, cx: &mut Context<SettingsView>) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .pb_2()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("settings.container_registries_desc").to_string()),
        )
        .child(
            Button::new("add-registry-btn")
                .primary()
                .small()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                .label(rust_i18n::t!("settings.add_registry").to_string())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.init_registry_edit(None, window, cx);
                })),
        )
        .into_any_element()
}

fn registry_card(
    reg: &transom_core::ContainerRegistry,
    p: &Palette,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let is_docker_hub = reg.id == "docker-hub";
    let is_default = reg.is_default;
    let auth_summary = if !reg.username.is_empty() {
        rust_i18n::t!("settings.registry_auth_configured", user = reg.username).to_string()
    } else {
        rust_i18n::t!("settings.registry_anonymous").to_string()
    };
    h_flex()
        .id(ElementId::Name(format!("reg-card-{}", reg.id).into()))
        .w_full()
        .p_3()
        .gap_3()
        .rounded(p.radius.lg)
        .border_1()
        .border_color(p.stroke.subtle)
        .bg(p.muted.opacity(0.03))
        .hover(|s| s.bg(p.muted.opacity(0.06)))
        .items_center()
        .justify_between()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .items_center()
                .child(registry_icon(is_docker_hub, p))
                .child(registry_identity(
                    reg,
                    is_docker_hub,
                    is_default,
                    auth_summary,
                    p,
                )),
        )
        .child(registry_actions(&reg.id, is_docker_hub, is_default, cx))
        .into_any_element()
}

fn registry_icon(is_docker_hub: bool, p: &Palette) -> AnyElement {
    v_flex()
        .w(px(36.0))
        .h(px(36.0))
        .rounded(p.radius.md)
        .bg(if is_docker_hub {
            p.tone.primary_subtle
        } else {
            p.surface.faint
        })
        .items_center()
        .justify_center()
        .child(
            AppIconElement::new(if is_docker_hub {
                AppIcon::Custom(CustomIcon::DOCKER)
            } else {
                AppIcon::Domain(DomainIcon::SSH)
            })
            .with_size(px(18.0))
            .text_color(if is_docker_hub {
                p.primary
            } else {
                p.muted_foreground
            }),
        )
        .into_any_element()
}

fn registry_identity(
    reg: &transom_core::ContainerRegistry,
    is_docker_hub: bool,
    is_default: bool,
    auth_summary: String,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(reg.name.clone()),
                )
                .when(is_docker_hub, |this| this.child(registry_official_tag(p)))
                .when(is_default, |this| this.child(registry_default_tag(p))),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .text_color(p.muted_foreground)
                        .child(reg.url.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.text.subtle)
                        .child(auth_summary),
                ),
        )
        .into_any_element()
}

fn registry_official_tag(p: &Palette) -> AnyElement {
    div()
        .px_1p5()
        .py_0p5()
        .text_xs()
        .rounded(p.radius.sm)
        .bg(p.muted.opacity(0.10))
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("settings.registry_official_tag").to_string())
        .into_any_element()
}

fn registry_default_tag(p: &Palette) -> AnyElement {
    h_flex()
        .gap_1()
        .items_center()
        .px_1p5()
        .py_0p5()
        .rounded(p.radius.full)
        .bg(p.tone.success_subtle)
        .border_1()
        .border_color(p.success.opacity(0.25))
        .child(
            div()
                .w(px(5.0))
                .h(px(5.0))
                .rounded(p.radius.full)
                .bg(p.success),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.success)
                .child(rust_i18n::t!("settings.registry_default").to_string()),
        )
        .into_any_element()
}

fn registry_actions(
    reg_id: &str,
    is_docker_hub: bool,
    is_default: bool,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let id_for_default = reg_id.to_string();
    let id_for_edit = reg_id.to_string();
    let id_for_delete = reg_id.to_string();
    h_flex()
        .gap_1p5()
        .items_center()
        .when(!is_default, |this| {
            this.child(
                Button::new(ElementId::Name(
                    format!("set-default-{}", id_for_default).into(),
                ))
                .ghost()
                .small()
                .label(rust_i18n::t!("settings.registry_set_default").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_default_registry(&id_for_default, cx);
                })),
            )
        })
        .child(
            Button::new(ElementId::Name(format!("edit-reg-{}", id_for_edit).into()))
                .secondary()
                .small()
                .label(rust_i18n::t!("settings.registry_edit").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.init_registry_edit(Some(id_for_edit.clone()), window, cx);
                })),
        )
        .when(!is_docker_hub, |this| {
            this.child(
                Button::new(ElementId::Name(format!("del-reg-{}", id_for_delete).into()))
                    .ghost()
                    .small()
                    .label(rust_i18n::t!("settings.registry_delete").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_registry(&id_for_delete, cx);
                    })),
            )
        })
        .into_any_element()
}
