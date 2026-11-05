//! 偏好设置中容器镜像仓库内联编辑与新建表单渲染。
//!
//! 设计原理（Why）：
//! - 将镜像仓库的新建/编辑内联表单渲染独立拆分，保证各子模块代码量精简内聚；
//! - 采用标准表单控件、开关与错误提示横幅，提供一体化的内联编辑体验。

use super::super::SettingsView;
use super::super::modal_state::RegistryEditState;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl SettingsView {
    /// 渲染仓库内联编辑/新建表单。
    pub(super) fn render_registry_edit_form(
        &mut self,
        edit: RegistryEditState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_new = edit.editing_id.is_none();
        let form_title = if is_new {
            rust_i18n::t!("settings.add_registry").to_string()
        } else {
            rust_i18n::t!("settings.edit_registry").to_string()
        };

        let is_default = edit.is_default;
        let error = edit.error.clone();

        v_flex()
            .w_full()
            .p_4()
            .gap_3()
            .rounded(p.radius.lg)
            .border_1()
            .border_color(p.border.opacity(0.18))
            .bg(p.muted.opacity(0.04))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(form_title),
            )
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.registry_name").to_string(),
                    edit.name.clone(),
                    p,
                ),
                None,
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.registry_url").to_string(),
                    edit.url.clone(),
                    p,
                ),
                None,
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.registry_username").to_string(),
                    edit.username.clone(),
                    p,
                ),
                None,
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.registry_password").to_string(),
                    edit.password.clone(),
                    p,
                ),
                None,
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.registry_default",
                Some("settings.registry_default_desc"),
                Switch::new("toggle-registry-default")
                    .checked(is_default)
                    .on_click(cx.listener(move |this, &checked, _, cx| {
                        if let Some(edit) = this.state.registry_edit.as_mut() {
                            edit.is_default = checked;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .when_some(error, |this, err| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .p_2()
                        .rounded(p.radius.md)
                        .bg(p.danger.opacity(0.10))
                        .child(
                            Icon::new(IconName::CircleAlert)
                                .with_size(px(14.0))
                                .text_color(p.danger),
                        )
                        .child(div().text_xs().text_color(p.danger).child(err)),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("cancel-reg-edit-btn")
                            .ghost()
                            .small()
                            .label(rust_i18n::t!("settings.registry_cancel").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.registry_edit = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("save-reg-edit-btn")
                            .primary()
                            .small()
                            .label(rust_i18n::t!("settings.registry_save").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.submit_registry_edit(cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
