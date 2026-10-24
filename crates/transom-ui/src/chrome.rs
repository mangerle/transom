//! 窗口顶栏（Top Bar）渲染模块。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

impl AppShell {
    pub(crate) fn render_top_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let is_maximized = window.is_maximized();
        let max_icon = if is_maximized {
            IconName::WindowRestore
        } else {
            IconName::WindowMaximize
        };
        let max_tooltip = if is_maximized {
            rust_i18n::t!("app.restore").to_string()
        } else {
            rust_i18n::t!("app.maximize").to_string()
        };

        let min_button = div()
            .id("window-control-minimize")
            .size(px(30.0))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(p.muted_foreground)
            .hover(|s| s.bg(p.sidebar_accent).text_color(p.foreground))
            .active(|s| s.bg(p.border))
            .tooltip(move |_window, cx| {
                Tooltip::new(rust_i18n::t!("app.minimize").to_string()).build(_window, cx)
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                window.minimize_window();
            })
            .child(Icon::new(IconName::WindowMinimize).small());

        let max_button = div()
            .id("window-control-maximize")
            .size(px(30.0))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(p.muted_foreground)
            .hover(|s| s.bg(p.sidebar_accent).text_color(p.foreground))
            .active(|s| s.bg(p.border))
            .tooltip(move |_window, cx| Tooltip::new(max_tooltip.clone()).build(_window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                window.zoom_window();
            })
            .child(Icon::new(max_icon).small());

        let close_button = div()
            .id("window-control-close")
            .size(px(30.0))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(p.muted_foreground)
            .hover(|s| s.bg(p.danger_hover).text_color(p.danger_foreground))
            .active(|s| s.bg(p.danger_active))
            .tooltip(move |_window, cx| {
                Tooltip::new(rust_i18n::t!("app.close").to_string()).build(_window, cx)
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.close_app(window, cx);
            }))
            .child(Icon::new(IconName::WindowClose).small());

        h_flex()
            .w_full()
            .h(px(36.0))
            .items_center()
            .justify_between()
            .px_3()
            .bg(p.title_bar)
            .border_b_1()
            .border_color(p.title_bar_border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::Terminal).text_color(p.sidebar_accent_foreground))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("app.name").to_string()),
                    ),
            )
            .child(div().flex_1().h_full().on_mouse_down(
                MouseButton::Left,
                |event: &MouseDownEvent, window, _| {
                    if event.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                },
            ))
            .child(
                h_flex()
                    .h_full()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .child(min_button)
                    .child(max_button)
                    .child(close_button),
            )
            .into_any_element()
    }
}
