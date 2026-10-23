//! 左侧活动栏（Activity Bar）与窗口顶栏（Top Bar）渲染模块。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::Connection;

/// 活动栏（Activity Bar）导航按钮属性结构体。
///
/// 遵循参数对象模式（Parameter Object Pattern），聚合按钮渲染所需属性，避免方法参数过多。
pub(crate) struct ActivityButtonProps {
    pub(crate) id: &'static str,
    pub(crate) icon: IconName,
    pub(crate) tooltip: String,
    pub(crate) selected: bool,
    pub(crate) disabled: bool,
}

impl AppShell {
    pub(crate) fn render_activity_button(
        props: ActivityButtonProps,
        p: &Palette,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut btn = div()
            .id(props.id)
            .size(px(34.0))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_window, cx| Tooltip::new(props.tooltip.clone()).build(_window, cx));

        if props.selected {
            btn = btn
                .bg(p.sidebar_accent)
                .text_color(p.sidebar_accent_foreground)
                .cursor_pointer();
        } else if props.disabled {
            btn = btn
                .text_color(p.muted_foreground.opacity(0.4))
                .cursor_not_allowed();
        } else {
            btn = btn
                .cursor_pointer()
                .text_color(p.muted_foreground)
                .hover(|s| {
                    s.bg(p.sidebar_accent.opacity(0.8))
                        .text_color(p.sidebar_foreground)
                })
                .active(|s| s.bg(p.sidebar_accent));
        }

        if !props.disabled {
            btn = btn.on_click(cx.listener(move |this, _, window, cx| {
                on_click(this, window, cx);
            }));
        }

        btn.child(Icon::new(props.icon).small()).into_any_element()
    }

    pub(crate) fn render_activity_bar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let has_ssh_tab = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| matches!(tab.session.connection, Connection::Ssh(_)));
        let show_forwards = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.forwards_panel.is_some());
        let show_sftp = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .is_some_and(|tab| tab.sftp.is_some());

        v_flex()
            .w(px(48.0))
            .flex_none()
            .h_full()
            .items_center()
            .justify_between()
            .py_3()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.border)
            .child(
                v_flex()
                    .gap_3()
                    .items_center()
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-sessions",
                            icon: IconName::Server,
                            tooltip: rust_i18n::t!("app.sessions").to_string(),
                            selected: self.sidebar_visible,
                            disabled: false,
                        },
                        p,
                        |this, _, cx| {
                            this.sidebar_visible = !this.sidebar_visible;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-quick-open",
                            icon: IconName::Search,
                            tooltip: rust_i18n::t!("app.quick_open").to_string(),
                            selected: false,
                            disabled: false,
                        },
                        p,
                        |this, window, cx| {
                            this.toggle_quick_open(window, cx);
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-forwards",
                            icon: IconName::ArrowLeftRight,
                            tooltip: rust_i18n::t!("forward.title").to_string(),
                            selected: show_forwards,
                            disabled: !has_ssh_tab,
                        },
                        p,
                        |this, window, cx| {
                            this.toggle_forward_panel(window, cx);
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-sftp",
                            icon: IconName::Folder,
                            tooltip: rust_i18n::t!("sftp.title").to_string(),
                            selected: show_sftp,
                            disabled: !has_ssh_tab,
                        },
                        p,
                        |this, window, cx| {
                            this.toggle_sftp(window, cx);
                        },
                        cx,
                    )),
            )
            .child(
                v_flex()
                    .gap_3()
                    .items_center()
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-language",
                            icon: IconName::Globe,
                            tooltip: if self.settings.language == "zh-CN" {
                                "切换为英文 (Switch to English)".to_string()
                            } else {
                                "切换为中文 (Switch to Chinese)".to_string()
                            },
                            selected: false,
                            disabled: false,
                        },
                        p,
                        |this, _, cx| {
                            this.toggle_language(cx);
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-import-ssh-config",
                            icon: IconName::Download,
                            tooltip: rust_i18n::t!("app.import_ssh_config").to_string(),
                            selected: false,
                            disabled: false,
                        },
                        p,
                        |this, _, cx| {
                            this.import_ssh_config(cx);
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-known-hosts",
                            icon: IconName::ShieldCheck,
                            tooltip: rust_i18n::t!("known_hosts.title").to_string(),
                            selected: false,
                            disabled: false,
                        },
                        p,
                        |this, _, cx| {
                            this.show_known_hosts_modal = true;
                            this.refresh_known_hosts();
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-keys",
                            icon: IconName::KeyRound,
                            tooltip: rust_i18n::t!("keys.title").to_string(),
                            selected: self.key_wizard.is_some(),
                            disabled: false,
                        },
                        p,
                        |this, window, cx| {
                            this.open_key_wizard(window, cx);
                        },
                        cx,
                    ))
                    .child(Self::render_activity_button(
                        ActivityButtonProps {
                            id: "nav-settings",
                            icon: IconName::Settings,
                            tooltip: rust_i18n::t!("settings.title").to_string(),
                            selected: false,
                            disabled: false,
                        },
                        p,
                        |this, window, cx| {
                            this.open_settings_modal(window, cx);
                        },
                        cx,
                    )),
            )
            .into_any_element()
    }

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
            .hover(|s| s.bg(rgb(0xef4444)).text_color(white()))
            .active(|s| s.bg(rgb(0xdc2626)))
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
