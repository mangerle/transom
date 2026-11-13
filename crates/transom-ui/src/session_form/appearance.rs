//! 会话表单的外观选择行：专属图标与环境色。
//!
//! 单独成文件是因为这两行要维护两套交互：图标是一排图标按钮，颜色是一排色
//! 块；混进主表单视图会让那个文件继续变大，两套控件的样式也各写各的。

use super::types::{ConnectionChoice, SessionForm};
use crate::palette::Palette;
use crate::session_appearance::{
    SESSION_COLOR_CHOICES, SESSION_ICON_CHOICES, session_color, session_icon,
};
use crate::session_form::SessionFormView;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{SessionColor, SessionIcon};
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon, UiIcon};

/// 色块直径：够点中，又不会让一排七个占满整行。
const SWATCH_SIZE: f32 = 18.0;

impl SessionFormView {
    /// 紧凑方形“图标”选择按钮：显示当前图标与环境色，点击弹出图标与颜色选择面板。
    pub(super) fn render_session_icon_picker_button(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = form.icon;
        let color = form.color;
        let display_icon = session_icon(icon).unwrap_or(match form.connection {
            ConnectionChoice::Local => AppIcon::Domain(DomainIcon::TERMINAL),
            _ => AppIcon::Ui(UiIcon::WEB_SERVER),
        });
        let icon_tint = session_color(color).unwrap_or(p.primary);
        let content_entity = cx.entity();
        let content_palette = p.clone();

        let popover = Popover::new("session-form-icon-popover")
            .anchor(Anchor::TopLeft)
            .offset(px(4.0))
            .open(form.icon_open)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                this.set_icon_open(*open, cx);
            }))
            .trigger(
                Button::new("session-form-icon-trigger")
                    .outline()
                    .child(
                        AppIconElement::new(display_icon)
                            .small()
                            .text_color(icon_tint),
                    )
                    .accessibility_label(rust_i18n::t!("form.icon").to_string()),
            )
            .content(move |_, _, _cx| {
                Self::render_session_appearance_popover(
                    &content_entity,
                    icon,
                    color,
                    &content_palette,
                )
            });

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.icon").to_string()),
            )
            .child(popover)
            .into_any_element()
    }

    /// 弹层内的图标与环境色选择面板。
    fn render_session_appearance_popover(
        entity: &Entity<SessionFormView>,
        icon: SessionIcon,
        color: SessionColor,
        p: &Palette,
    ) -> AnyElement {
        let icon_row = Self::render_session_icon_row_with_entity(entity, icon);
        let color_row = Self::render_session_color_row_with_entity(entity, color, p);

        v_flex()
            .w(rems(16.5))
            .p_2()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.icon").to_string()),
            )
            .child(icon_row)
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.color").to_string()),
            )
            .child(color_row)
            .into_any_element()
    }

    /// 会话图标选择行：自动图标加十个预设图标。
    fn render_session_icon_row_with_entity(
        entity: &Entity<SessionFormView>,
        icon: SessionIcon,
    ) -> AnyElement {
        let auto_entity = entity.clone();
        let icon_row = h_flex()
            .gap_1()
            .flex_wrap()
            .child(
                Button::new("session-icon-auto")
                    .small()
                    .child(AppIconElement::new(UiIcon::REFRESH_AUTO).small())
                    .selected(icon == SessionIcon::Auto)
                    .accessibility_label(rust_i18n::t!("form.icon_auto").to_string())
                    .tooltip(rust_i18n::t!("form.icon_auto").to_string())
                    .on_click(move |_: &ClickEvent, _, cx| {
                        auto_entity.update(cx, |this, cx| {
                            this.edit(cx, |form| {
                                form.icon = SessionIcon::Auto;
                                form.icon_open = false;
                            });
                        });
                    }),
            )
            .children(SESSION_ICON_CHOICES.into_iter().map(|choice| {
                let Some(name) = session_icon(choice) else {
                    return div().into_any_element();
                };
                let pick_entity = entity.clone();
                Button::new(ElementId::Name(format!("session-icon-{choice:?}").into()))
                    .small()
                    .child(AppIconElement::new(name).small())
                    .accessibility_label(format!("{choice:?}"))
                    .tooltip(format!("{choice:?}"))
                    .selected(icon == choice)
                    .on_click(move |_: &ClickEvent, _, cx| {
                        pick_entity.update(cx, |this, cx| {
                            this.edit(cx, |form| {
                                form.icon = choice;
                                form.icon_open = false;
                            });
                        });
                    })
                    .into_any_element()
            }));
        icon_row.into_any_element()
    }

    /// 会话环境色选择行：无色加预设色板。
    fn render_session_color_row_with_entity(
        entity: &Entity<SessionFormView>,
        color: SessionColor,
        p: &Palette,
    ) -> AnyElement {
        let none_entity = entity.clone();
        let color_row = h_flex()
            .gap_2()
            .child(
                div()
                    .id(ElementId::Name("session-color-none".into()))
                    .size(px(SWATCH_SIZE))
                    .rounded_full()
                    .border_1()
                    .border_color(if color == SessionColor::None {
                        p.primary
                    } else {
                        p.border
                    })
                    .bg(p.background)
                    .cursor_pointer()
                    .tooltip(move |_window, cx| {
                        Tooltip::new(rust_i18n::t!("form.color_none").to_string())
                            .build(_window, cx)
                    })
                    .on_click(move |_: &ClickEvent, _, cx| {
                        none_entity.update(cx, |this, cx| {
                            this.edit(cx, |form| {
                                form.color = SessionColor::None;
                                form.icon_open = false;
                            });
                        });
                    })
                    .into_any_element(),
            )
            .children(SESSION_COLOR_CHOICES.into_iter().map(|choice| {
                let Some(value) = session_color(choice) else {
                    return div().into_any_element();
                };
                let pick_entity = entity.clone();
                div()
                    .id(ElementId::Name(format!("session-color-{choice:?}").into()))
                    .size(px(SWATCH_SIZE))
                    .rounded_full()
                    .bg(value)
                    .border_1()
                    .border_color(if color == choice {
                        p.foreground
                    } else {
                        p.border
                    })
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, _, cx| {
                        pick_entity.update(cx, |this, cx| {
                            this.edit(cx, |form| {
                                form.color = choice;
                                form.icon_open = false;
                            });
                        });
                    })
                    .into_any_element()
            }));
        color_row.into_any_element()
    }
}
