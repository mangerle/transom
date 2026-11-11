//! 会话表单的外观选择行：专属图标与环境色。
//!
//! 单独成文件是因为这两行要维护两套交互：图标是一排图标按钮，颜色是一排色
//! 块；混进主表单视图会让那个文件继续变大，两套控件的样式也各写各的。

use crate::palette::Palette;
use crate::session_appearance::{
    SESSION_COLOR_CHOICES, SESSION_ICON_CHOICES, session_color, session_icon,
};
use crate::session_form::SessionFormView;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{SessionColor, SessionIcon};
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 色块直径：够点中，又不会让一排七个占满整行。
const SWATCH_SIZE: f32 = 18.0;

impl SessionFormView {
    /// 外观选择行：图标一排、环境色一排。
    ///
    /// `default_icon` 是"跟随连接类型"时显示的图标，由调用方按当前连接类型给出，
    /// 这样用户在按钮上就能看到最终会画成什么样。
    pub(super) fn render_session_appearance(
        &self,
        icon: SessionIcon,
        color: SessionColor,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon_row = self.render_session_icon_row(icon, cx);
        let color_row = self.render_session_color_row(color, p, cx);

        v_flex()
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
    fn render_session_icon_row(&self, icon: SessionIcon, cx: &mut Context<Self>) -> AnyElement {
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
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.edit(cx, |form| form.icon = SessionIcon::Auto);
                    })),
            )
            .children(SESSION_ICON_CHOICES.into_iter().map(|choice| {
                let Some(name) = session_icon(choice) else {
                    return div().into_any_element();
                };
                Button::new(ElementId::Name(format!("session-icon-{choice:?}").into()))
                    .small()
                    .child(AppIconElement::new(name).small())
                    .accessibility_label(format!("{choice:?}"))
                    .tooltip(format!("{choice:?}"))
                    .selected(icon == choice)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.edit(cx, |form| form.icon = choice);
                    }))
                    .into_any_element()
            }));
        icon_row.into_any_element()
    }

    /// 会话环境色选择行：无色加预设色板。
    fn render_session_color_row(
        &self,
        color: SessionColor,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.edit(cx, |form| form.color = SessionColor::None);
                    }))
                    .into_any_element(),
            )
            .children(SESSION_COLOR_CHOICES.into_iter().map(|choice| {
                let Some(value) = session_color(choice) else {
                    return div().into_any_element();
                };
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
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.edit(cx, |form| form.color = choice);
                    }))
                    .into_any_element()
            }));
        color_row.into_any_element()
    }
}
