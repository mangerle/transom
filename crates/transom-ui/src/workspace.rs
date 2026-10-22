//! 主工作区标签栏、终端容器、断线重连提示与底部硬件状态监控栏渲染模块。

use crate::app::{AppShell, TabStatus};
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, TabId};

impl AppShell {
    pub(crate) fn render_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut tab_strip = h_flex()
            .h(px(32.0))
            .items_center()
            .bg(p.tab_bar)
            .border_b_1()
            .border_color(p.border)
            .overflow_hidden();
        let app_entity = cx.entity().clone();
        for tab in &self.tabs {
            let tab_id = tab.id;
            let session_id = tab.session_id;
            let is_local = matches!(tab.session.connection, Connection::Local(_));
            let active = self.active_tab == Some(tab_id);
            let dot_color = match &tab.status {
                TabStatus::Connecting => p.muted_foreground,
                TabStatus::Connected => p.success,
                TabStatus::Failed(_) | TabStatus::Disconnected => p.danger,
            };
            let mut tab_element = h_flex()
                .id(ElementId::Name(format!("tab-{tab_id}").into()))
                .h_full()
                .px_2p5()
                .gap_1p5()
                .items_center()
                .max_w(px(200.0))
                .min_w(px(100.0))
                .overflow_hidden()
                .border_r_1()
                .border_color(p.border)
                .cursor_pointer();
            if active {
                tab_element = tab_element
                    .bg(p.tab_active)
                    .text_color(p.tab_active_foreground);
            } else {
                tab_element = tab_element.text_color(p.tab_foreground).hover(|style| {
                    style
                        .bg(p.sidebar_accent.opacity(0.6))
                        .text_color(p.foreground)
                });
            }
            let tab_icon = match &tab.session.connection {
                Connection::Local(_) => IconName::Terminal,
                _ => IconName::Server,
            };
            let tab_title = tab.title.clone();
            let mut element = tab_element
                .tooltip(move |_window, cx| Tooltip::new(tab_title.clone()).build(_window, cx))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.activate_tab(tab_id, window, cx)),
                )
                .context_menu({
                    let app_entity = app_entity.clone();
                    move |menu, _window, _cx| {
                        let app = app_entity.clone();
                        menu.item(
                            PopupMenuItem::new(rust_i18n::t!("menu.reconnect").to_string())
                                .icon(IconName::RefreshCw)
                                .on_click({
                                    let app = app.clone();
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.reconnect_tab(tab_id, window, cx);
                                        });
                                    }
                                }),
                        )
                        .item(
                            PopupMenuItem::new(rust_i18n::t!("menu.duplicate").to_string())
                                .icon(IconName::Plus)
                                .on_click({
                                    let app = app.clone();
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            if is_local {
                                                this.open_local_terminal(window, cx);
                                            } else {
                                                this.connect_session(session_id, window, cx);
                                            }
                                        });
                                    }
                                }),
                        )
                        .item(PopupMenuItem::separator())
                        .item(
                            PopupMenuItem::new(rust_i18n::t!("menu.close_tab").to_string())
                                .icon(IconName::X)
                                .on_click({
                                    let app = app.clone();
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.close_tab(tab_id, window, cx);
                                        });
                                    }
                                }),
                        )
                        .item(
                            PopupMenuItem::new(rust_i18n::t!("menu.close_other_tabs").to_string())
                                .icon(IconName::Trash)
                                .on_click({
                                    let app = app.clone();
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.close_other_tabs(tab_id, window, cx);
                                        });
                                    }
                                }),
                        )
                    }
                })
                .child(div().size(px(8.0)).rounded_full().flex_none().bg(dot_color))
                .child(Icon::new(tab_icon).flex_none().text_color(if active {
                    p.tab_active_foreground
                } else {
                    p.muted_foreground
                }))
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .truncate()
                        .child(tab.title.clone()),
                );
            if self.settings.window.show_tab_close_button {
                element = element.child(
                    div()
                        .id(ElementId::Name(format!("close-{tab_id}").into()))
                        .size(px(18.0))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_color(if active {
                            p.tab_active_foreground
                        } else {
                            p.muted_foreground
                        })
                        .hover(|s| {
                            s.bg(p.muted_foreground.opacity(0.2))
                                .text_color(p.foreground)
                        })
                        .active(|s| s.bg(p.muted_foreground.opacity(0.3)))
                        .tooltip(move |_window, cx| {
                            Tooltip::new(rust_i18n::t!("app.close").to_string()).build(_window, cx)
                        })
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(tab_id, window, cx);
                        }))
                        .child(Icon::new(IconName::WindowClose).xsmall()),
                );
            }
            tab_strip = tab_strip.child(element);
        }

        tab_strip = tab_strip.child(
            div().h_full().px_1p5().flex().items_center().child(
                div()
                    .id("tab-strip-quick-new")
                    .size(px(22.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(p.muted_foreground)
                    .hover(|s| s.bg(p.sidebar_accent).text_color(p.foreground))
                    .active(|s| s.bg(p.border))
                    .tooltip(move |_window, cx| {
                        Tooltip::new(rust_i18n::t!("app.new_local").to_string()).build(_window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_local_terminal(window, cx);
                    }))
                    .child(Icon::new(IconName::Plus).xsmall()),
            ),
        );

        let disconnected_tab = self.active_tab.and_then(|tab_id| {
            self.tabs
                .iter()
                .find(|tab| tab.id == tab_id && matches!(&tab.status, TabStatus::Disconnected))
                .map(|tab| tab.id)
        });

        let content = if let Some(tab_id) = self.active_tab {
            if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
                if let Some(terminal) = self.tabs[index].terminal.clone() {
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(terminal)
                        .into_any_element()
                } else {
                    self.render_tab_status(index, window, cx, p)
                }
            } else {
                self.render_empty_state(p, cx)
            }
        } else {
            self.render_empty_state(p, cx)
        };

        let mut main = v_flex()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(p.background)
            .child(tab_strip);
        if let Some(tab_id) = disconnected_tab {
            main = main.child(self.render_reconnect_banner(tab_id, p, cx));
        }
        main.child(content).into_any_element()
    }

    pub(crate) fn render_reconnect_banner(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_3()
            .py_1()
            .gap_3()
            .items_center()
            .justify_between()
            .bg(p.muted)
            .text_color(p.muted_foreground)
            .child(
                div()
                    .text_xs()
                    .child(rust_i18n::t!("tab.disconnected").to_string()),
            )
            .child(
                Button::new(ElementId::Name(format!("banner-reconnect-{tab_id}").into()))
                    .primary()
                    .xsmall()
                    .label(rust_i18n::t!("common.reconnect").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.reconnect_tab(tab_id, window, cx)
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_tab_status(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let tab_id = self.tabs[index].id;
        let status = &self.tabs[index].status;
        let (label, detail) = match status {
            TabStatus::Connecting => (
                rust_i18n::t!("tab.connecting").to_string(),
                rust_i18n::t!("status.connecting").to_string(),
            ),
            TabStatus::Connected => (rust_i18n::t!("tab.connected").to_string(), String::new()),
            TabStatus::Disconnected => (
                rust_i18n::t!("tab.disconnected").to_string(),
                rust_i18n::t!("status.disconnected").to_string(),
            ),
            TabStatus::Failed(error) => (
                rust_i18n::t!("tab.failed").to_string(),
                localized_error(error),
            ),
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .child(div().text_lg().child(label))
            .child(div().text_sm().text_color(p.muted_foreground).child(detail))
            .child(
                Button::new(ElementId::Name(format!("reconnect-{tab_id}").into()))
                    .primary()
                    .label(rust_i18n::t!("common.reconnect").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.reconnect_tab(tab_id, window, cx)
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_empty_state(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Icon::new(IconName::Terminal).text_color(p.muted_foreground))
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("app.no_tab").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("app.no_tab_hint").to_string()),
            )
            .child(
                Button::new("empty-new-session")
                    .primary()
                    .small()
                    .icon(IconName::Plus)
                    .label(rust_i18n::t!("app.new_session").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_new_session(window, cx);
                    })),
            )
            .into_any_element()
    }
}
