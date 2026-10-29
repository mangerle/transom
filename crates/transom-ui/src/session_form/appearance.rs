//! 会话表单的外观选择行：专属图标与环境色。
//!
//! 单独成文件是因为这两行要维护两套交互：图标是一排图标按钮，颜色是一排色
//! 块；混进主表单视图会让那个文件继续变大，两套控件的样式也各写各的。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_appearance::{
    SESSION_COLOR_CHOICES, SESSION_ICON_CHOICES, session_color, session_icon,
};
use crate::session_form::ConnectionChoice;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{SessionColor, SessionIcon};

/// 色块直径：够点中，又不会让一排七个占满整行。
const SWATCH_SIZE: f32 = 18.0;

/// 连接类型对应的默认图标：表单里的"跟随连接类型"选项按它显示。
pub(crate) fn default_icon_for(connection: ConnectionChoice) -> IconName {
    match connection {
        ConnectionChoice::Ssh => IconName::Server,
        ConnectionChoice::Serial => IconName::Cable,
        ConnectionChoice::Telnet => IconName::Network,
        ConnectionChoice::Mosh => IconName::Radio,
    }
}

impl AppShell {
    /// 外观选择行：图标一排、环境色一排。
    ///
    /// `default_icon` 是"跟随连接类型"时显示的图标，由调用方按当前连接类型给出，
    /// 这样用户在按钮上就能看到最终会画成什么样。
    ///
    /// 刻意做成不借用 `self` 的关联函数：表单视图此刻正持有 `self.modals.form` 的
    /// 不可变借用，再借 `&mut self` 会与它冲突。
    pub(crate) fn render_session_appearance(
        default_icon: IconName,
        icon: SessionIcon,
        color: SessionColor,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon_row = h_flex()
            .gap_1()
            .child(
                Button::new("session-icon-auto")
                    .small()
                    .icon(default_icon)
                    .selected(icon == SessionIcon::Auto)
                    .tooltip(rust_i18n::t!("form.icon_auto").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(form) = this.modals.form.as_mut() {
                            form.icon = SessionIcon::Auto;
                        }
                        cx.notify();
                    })),
            )
            .children(SESSION_ICON_CHOICES.into_iter().map(|choice| {
                let Some(name) = session_icon(choice) else {
                    return div().into_any_element();
                };
                Button::new(ElementId::Name(format!("session-icon-{choice:?}").into()))
                    .small()
                    .icon(name)
                    .selected(icon == choice)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(form) = this.modals.form.as_mut() {
                            form.icon = choice;
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }));
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(form) = this.modals.form.as_mut() {
                            form.color = SessionColor::None;
                        }
                        cx.notify();
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(form) = this.modals.form.as_mut() {
                            form.color = choice;
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }));
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
}
