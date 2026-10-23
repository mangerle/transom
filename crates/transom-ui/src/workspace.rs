//! 主工作区标签栏、分屏窗格、断线重连提示与底部硬件状态监控栏渲染模块。

use crate::app::{AppShell, TabStatus};
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, SplitAxis, TabId};

impl AppShell {
    pub(crate) fn render_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let disconnected_tab = self.active_tab.and_then(|tab_id| {
            self.tabs
                .iter()
                .find(|tab| tab.id == tab_id && matches!(&tab.status, TabStatus::Disconnected))
                .map(|tab| tab.id)
        });
        let content = self.render_active_content(window, cx, p);

        let mut main = v_flex()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(p.background);
        if !self.immersive {
            main = main.child(self.render_tab_strip(p, cx));
        }
        if let Some(tab_id) = disconnected_tab {
            main = main.child(self.render_reconnect_banner(tab_id, p, cx));
        }
        main.child(content).into_any_element()
    }

    /// 按标签组渲染标签栏：一个标签组只显示一个条目，标题取活动窗格。
    fn render_tab_strip(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_active_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(group_index) = self.active_group_index() else {
            return self.render_empty_state(p, cx);
        };
        let pane_ids = self.groups[group_index].panes.clone();
        let axis = self.groups[group_index].axis;
        let active_pane = self.groups[group_index].active_pane;
        if pane_ids.len() == 1 {
            let Some(index) = self.tabs.iter().position(|tab| tab.id == pane_ids[0]) else {
                return self.render_empty_state(p, cx);
            };
            return self.render_pane_body(index, window, cx, p);
        }

        let mut layout = match axis {
            SplitAxis::Horizontal => h_flex(),
            SplitAxis::Vertical => v_flex(),
        }
        .flex_1()
        .size_full()
        .overflow_hidden();
        for (position, pane_id) in pane_ids.into_iter().enumerate() {
            if position > 0 {
                layout = layout.child(match axis {
                    SplitAxis::Horizontal => {
                        div().w(px(1.0)).h_full().bg(p.border).into_any_element()
                    }
                    SplitAxis::Vertical => {
                        div().h(px(1.0)).w_full().bg(p.border).into_any_element()
                    }
                });
            }
            let Some(index) = self.tabs.iter().position(|tab| tab.id == pane_id) else {
                continue;
            };
            let pane = self.render_split_pane(index, pane_id == active_pane, window, cx, p);
            layout = layout.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.activate_tab(pane_id, window, cx);
                        }),
                    )
                    .child(pane),
            );
        }
        div()
            .flex_1()
            .size_full()
            .overflow_hidden()
            .child(layout)
            .into_any_element()
    }

    fn render_pane_body(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        if let Some(terminal) = self.tabs[index].terminal.clone() {
            div()
                .flex_1()
                .overflow_hidden()
                .child(terminal)
                .into_any_element()
        } else {
            self.render_tab_status(index, window, cx, p)
        }
    }

    fn render_split_pane(
        &mut self,
        index: usize,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let tab_id = self.tabs[index].id;
        let title = self.tabs[index].title.clone();
        let (dot_color, icon_name) = match &self.tabs[index].status {
            TabStatus::Connecting => (p.muted_foreground, IconName::Loader),
            TabStatus::Connected => (p.success, IconName::Terminal),
            TabStatus::Failed(_) | TabStatus::Disconnected => (p.danger, IconName::CircleAlert),
        };
        let header = h_flex()
            .id(ElementId::Name(format!("pane-header-{tab_id}").into()))
            .h(px(28.0))
            .flex_none()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(p.border)
            .bg(if active { p.tab_active } else { p.tab_bar })
            .text_color(if active {
                p.tab_active_foreground
            } else {
                p.tab_foreground
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_tab(tab_id, window, cx);
            }))
            .child(div().size(px(7.0)).rounded_full().flex_none().bg(dot_color))
            .child(Icon::new(icon_name).xsmall())
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .child(title),
            )
            .child(
                div()
                    .id(ElementId::Name(format!("close-pane-{tab_id}").into()))
                    .size(px(18.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(p.muted_foreground.opacity(0.2))
                            .text_color(p.foreground)
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_pane(tab_id, window, cx);
                    }))
                    .child(Icon::new(IconName::X).xsmall()),
            );
        let body = self.render_pane_body(index, window, cx, p);
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element()
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
