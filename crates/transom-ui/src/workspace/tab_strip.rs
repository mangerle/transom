//! 标签栏渲染。

use crate::app::{AppShell, TabStatus};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, SplitAxis};

impl AppShell {
    pub(super) fn render_tab_strip(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut tab_strip = h_flex()
            .h(px(32.0))
            .items_center()
            .bg(p.tab_bar)
            .border_b_1()
            .border_color(p.border)
            .overflow_hidden();
        let app_entity = cx.entity().clone();
        let can_split = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| !matches!(tab.session.connection, Connection::Serial(_)));

        for group in &self.groups {
            let Some(tab) = self.tabs.iter().find(|tab| tab.id == group.active_pane) else {
                continue;
            };
            let group_id = group.id;
            let active_pane = group.active_pane;
            let session_id = tab.session_id;
            let title = tab.title.clone();
            let pane_count = group.panes.len();
            let is_local = matches!(tab.session.connection, Connection::Local(_));
            let active = self.active_tab == Some(active_pane);
            let dot_color = match &tab.status {
                TabStatus::Connecting => p.muted_foreground,
                TabStatus::Connected => p.success,
                TabStatus::Failed(_) | TabStatus::Disconnected => p.danger,
            };
            let tab_icon = match &tab.session.connection {
                Connection::Local(_) => IconName::Terminal,
                Connection::Serial(_) => IconName::Cable,
                _ => IconName::Server,
            };
            let mut tab_element = h_flex()
                .id(ElementId::Name(format!("tab-{group_id}").into()))
                .h_full()
                .px_2p5()
                .gap_1p5()
                .items_center()
                .max_w(px(220.0))
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
            let tooltip_text = if pane_count > 1 {
                rust_i18n::t!("tab.pane_count", title = title, count = pane_count).to_string()
            } else {
                title.clone()
            };
            let mut element = tab_element
                .tooltip(move |_window, cx| Tooltip::new(tooltip_text.clone()).build(_window, cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.activate_tab(active_pane, window, cx);
                }))
                .context_menu({
                    let app_entity = app_entity.clone();
                    move |menu, _window, _cx| {
                        let app = app_entity.clone();
                        let mut menu = menu
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.reconnect").to_string())
                                    .icon(IconName::RefreshCw)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.reconnect_tab(active_pane, window, cx);
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
                                                this.activate_tab(active_pane, window, cx);
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
                                PopupMenuItem::new(rust_i18n::t!("menu.split_right").to_string())
                                    .icon(IconName::Columns2)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.split_active_pane(
                                                    SplitAxis::Horizontal,
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.split_down").to_string())
                                    .icon(IconName::Rows2)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.split_active_pane(
                                                    SplitAxis::Vertical,
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }),
                            );
                        if pane_count > 1 {
                            menu = menu
                                .item(
                                    PopupMenuItem::new(
                                        rust_i18n::t!("menu.toggle_split_axis").to_string(),
                                    )
                                    .icon(IconName::ArrowLeftRight)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.activate_tab(active_pane, window, cx);
                                                this.toggle_split_axis(cx);
                                            });
                                        }
                                    }),
                                )
                                .item(
                                    PopupMenuItem::new(
                                        rust_i18n::t!("menu.close_pane").to_string(),
                                    )
                                    .icon(IconName::X)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.close_pane(active_pane, window, cx);
                                            });
                                        }
                                    }),
                                );
                        }
                        menu.item(PopupMenuItem::separator())
                            .item(
                                PopupMenuItem::new(rust_i18n::t!("menu.close_tab").to_string())
                                    .icon(IconName::X)
                                    .on_click({
                                        let app = app.clone();
                                        move |_, window, cx| {
                                            app.update(cx, |this, cx| {
                                                this.close_group(group_id, window, cx);
                                            });
                                        }
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(
                                    rust_i18n::t!("menu.close_other_tabs").to_string(),
                                )
                                .icon(IconName::Trash)
                                .on_click({
                                    let app = app.clone();
                                    move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.activate_tab(active_pane, window, cx);
                                            this.close_other_tabs(active_pane, window, cx);
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
            if pane_count > 1 {
                element = element.child(
                    h_flex()
                        .flex_none()
                        .gap_0p5()
                        .items_center()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(
                            Icon::new(if group.axis == SplitAxis::Horizontal {
                                IconName::Columns2
                            } else {
                                IconName::Rows2
                            })
                            .xsmall(),
                        )
                        .child(pane_count.to_string()),
                );
            }
            if self.settings.window.show_tab_close_button {
                element = element.child(
                    div()
                        .id(ElementId::Name(format!("close-{group_id}").into()))
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
                            this.close_group(group_id, window, cx);
                        }))
                        .child(Icon::new(IconName::WindowClose).xsmall()),
                );
            }
            tab_strip = tab_strip.child(element);
        }

        tab_strip
            .child(
                div()
                    .h_full()
                    .px_1p5()
                    .flex()
                    .items_center()
                    .child(Self::tab_strip_button(
                        "tab-strip-quick-new",
                        IconName::Plus,
                        rust_i18n::t!("app.new_local").to_string(),
                        true,
                        p,
                        cx.listener(|this, _, window, cx| {
                            this.open_local_terminal(window, cx);
                        }),
                    )),
            )
            .child(
                div()
                    .h_full()
                    .px_1p5()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(Self::tab_strip_button(
                        "tab-strip-split-right",
                        IconName::Columns2,
                        rust_i18n::t!("menu.split_right").to_string(),
                        can_split,
                        p,
                        cx.listener(|this, _, window, cx| {
                            this.split_active_pane(SplitAxis::Horizontal, window, cx);
                        }),
                    ))
                    .child(Self::tab_strip_button(
                        "tab-strip-split-down",
                        IconName::Rows2,
                        rust_i18n::t!("menu.split_down").to_string(),
                        can_split,
                        p,
                        cx.listener(|this, _, window, cx| {
                            this.split_active_pane(SplitAxis::Vertical, window, cx);
                        }),
                    )),
            )
            .into_any_element()
    }

    fn tab_strip_button(
        id: &'static str,
        icon: IconName,
        tooltip: String,
        enabled: bool,
        p: &Palette,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let mut button = div()
            .id(id)
            .size(px(22.0))
            .rounded_sm()
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx));
        if enabled {
            button = button
                .cursor_pointer()
                .text_color(p.muted_foreground)
                .hover(|s| s.bg(p.sidebar_accent).text_color(p.foreground))
                .active(|s| s.bg(p.border))
                .on_click(on_click);
        } else {
            button = button
                .cursor_not_allowed()
                .text_color(p.muted_foreground.opacity(0.4));
        }
        button.child(Icon::new(icon).xsmall()).into_any_element()
    }
}
