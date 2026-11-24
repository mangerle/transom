//! RDP 视口视图的 GPUI 渲染层与悬浮工具栏、TLS 证书告警面板实现。

use super::toolbar::{RdpToolbarCallback, RdpToolbarParams, render_rdp_toolbar};
use super::view::{RdpView, RdpViewEvent, RdpViewState};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_rdp::{KeyChord, RdpInput};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

impl RdpView {
    /// 渲染状态提示视图（连接中、断开或失败，断开/失败提供一键重连入口）。
    pub(super) fn render_status_view(&self, p: &Palette, cx: &Context<Self>) -> AnyElement {
        let reconnect_button = || {
            Button::new(ElementId::Name("rdp-reconnect-btn".into()))
                .small()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                .label(rust_i18n::t!("common.reconnect").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.state = RdpViewState::Connecting;
                    cx.emit(RdpViewEvent::Reconnect);
                    cx.notify();
                }))
        };

        match &self.state {
            RdpViewState::Connecting => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("rdp.connecting").to_string())
                .into_any_element(),
            RdpViewState::HostKeyMismatch {
                host,
                port,
                previous_fingerprint,
                current_fingerprint,
            } => {
                let host_owned = host.clone();
                let port_val = *port;
                let fp_owned = current_fingerprint.clone();
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .child(
                        v_flex()
                            .max_w(px(520.0))
                            .gap_3()
                            .p_4()
                            .rounded(p.radius.lg)
                            .bg(p.surface.floating)
                            .border_1()
                            .border_color(p.danger)
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.danger)
                                    .child(
                                        rust_i18n::t!(
                                            "rdp.tls_warning_title",
                                            host = host,
                                            port = port
                                        )
                                        .to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("rdp.tls_warning_desc").to_string()),
                            )
                            .child(
                                div().text_xs().text_color(p.muted_foreground).child(
                                    rust_i18n::t!(
                                        "rdp.tls_previous_fingerprint",
                                        fingerprint = previous_fingerprint
                                    )
                                    .to_string(),
                                ),
                            )
                            .child(
                                div().text_xs().text_color(p.foreground).child(
                                    rust_i18n::t!(
                                        "rdp.tls_current_fingerprint",
                                        fingerprint = current_fingerprint
                                    )
                                    .to_string(),
                                ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .pt_1()
                                    .child(
                                        Button::new(ElementId::Name("rdp-trust-host-btn".into()))
                                            .small()
                                            .danger()
                                            .label(
                                                rust_i18n::t!("rdp.tls_trust_and_reconnect")
                                                    .to_string(),
                                            )
                                            .on_click(cx.listener(
                                                move |this, _: &ClickEvent, _, cx| {
                                                    this.state = RdpViewState::Connecting;
                                                    cx.emit(RdpViewEvent::TrustAndReconnect {
                                                        host: host_owned.clone(),
                                                        port: port_val,
                                                        fingerprint: fp_owned.clone(),
                                                    });
                                                    cx.notify();
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new(ElementId::Name("rdp-cancel-trust-btn".into()))
                                            .small()
                                            .ghost()
                                            .label(rust_i18n::t!("common.cancel").to_string())
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.state = RdpViewState::Disconnected;
                                                    cx.emit(RdpViewEvent::Disconnected);
                                                    cx.notify();
                                                },
                                            )),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            RdpViewState::Failed(msg) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(p.danger)
                .child(rust_i18n::t!("rdp.failed", msg = msg).to_string())
                .child(reconnect_button())
                .into_any_element(),
            RdpViewState::Disconnected => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("rdp.disconnected").to_string())
                .child(reconnect_button())
                .into_any_element(),
            RdpViewState::Connected { .. } => div().into_any_element(),
        }
    }

    /// 渲染悬浮工具栏（默认收起为顶部窄条，鼠标移入自动展开、移出自动收起，支持全屏与图钉固定）。
    pub(super) fn render_floating_toolbar(&self, p: &Palette, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity().clone();
        let make_action = |f: fn(&mut RdpView, &mut Window, &mut Context<RdpView>)| {
            let entity = view.clone();
            Box::new(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                entity.update(cx, |this, cx| f(this, window, cx));
            }) as RdpToolbarCallback
        };

        let show_full = self.toolbar_expanded || self.toolbar_pinned;
        let content = if show_full {
            render_rdp_toolbar(RdpToolbarParams {
                palette: p,
                is_muted: self.is_muted,
                is_scaled: self.is_scaled,
                is_fullscreen: self.is_fullscreen,
                is_pinned: self.toolbar_pinned,
                rx_bytes_per_sec: self.rx_bytes_per_sec,
                rtt_ms: self.rtt_ms,
                on_send_cad: make_action(|this, _, _| this.send_key_chord(KeyChord::CtrlAltDelete)),
                on_send_win: make_action(|this, _, _| this.send_key_chord(KeyChord::Win)),
                on_toggle_mute: make_action(|this, _, cx| {
                    this.is_muted = !this.is_muted;
                    this.send_input(RdpInput::MuteAudio(this.is_muted));
                    cx.notify();
                }),
                on_toggle_scale: make_action(|this, _, cx| {
                    this.is_scaled = !this.is_scaled;
                    if this.is_scaled {
                        this.input.pan_offset = (0.0, 0.0);
                    }
                    cx.notify();
                }),
                on_toggle_fullscreen: make_action(|this, window, cx| {
                    window.toggle_fullscreen();
                    this.is_fullscreen = !this.is_fullscreen;
                    this.toolbar_expanded = false;
                    cx.emit(RdpViewEvent::FullscreenChanged(this.is_fullscreen));
                    cx.notify();
                }),
                on_toggle_pin: make_action(|this, _, cx| {
                    this.toolbar_pinned = !this.toolbar_pinned;
                    cx.notify();
                }),
                on_disconnect: make_action(|this, window, cx| {
                    if this.is_fullscreen && window.is_fullscreen() {
                        window.toggle_fullscreen();
                    }
                    this.is_fullscreen = false;
                    this.disconnect();
                    cx.emit(RdpViewEvent::FullscreenChanged(false));
                    cx.emit(RdpViewEvent::Disconnected);
                    cx.notify();
                }),
            })
        } else {
            // 收起态：顶部正中极窄半透明抓手，不遮挡远端标题栏或浏览器标签栏
            div()
                .w(px(88.0))
                .h(px(6.0))
                .rounded_full()
                .bg(p.surface.floating)
                .border_1()
                .border_color(p.border)
                .opacity(0.75)
                .into_any_element()
        };

        div()
            .id("rdp-floating-toolbar-zone")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .pt(if show_full { px(8.0) } else { px(2.0) })
            .pb(px(6.0))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.toolbar_expanded != *hovered {
                    this.toolbar_expanded = *hovered;
                    cx.notify();
                }
            }))
            .child(content)
            .into_any_element()
    }

    /// 渲染绑定鼠标与滚轮事件的桌面画布容器，与悬浮工具栏物理隔离，防止工具栏点击穿透到远端。
    fn render_interactive_canvas(&self, cx: &mut Context<Self>) -> Div {
        let mut container = div()
            .size_full()
            .bg(gpui::black())
            .cursor(CursorStyle::Arrow)
            .on_mouse_move(cx.listener(|this, e, _, cx| this.handle_mouse_move(e, cx)))
            .on_mouse_exit(cx.listener(|this, _, _, cx| this.handle_mouse_exit(cx)))
            .on_scroll(cx.listener(|this, e, _, cx| this.handle_scroll(e, cx)))
            .on_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, window, cx| {
                this.handle_mouse_down(e, window, cx);
            }))
            .child(self.render_desktop_canvas());

        for btn in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Navigate(NavigationDirection::Back),
            MouseButton::Navigate(NavigationDirection::Forward),
        ] {
            container = container.on_mouse_up(
                btn,
                cx.listener(move |this, _, _, _| {
                    this.handle_mouse_up(btn);
                }),
            );
        }

        container
    }
}

impl Focusable for RdpView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RdpView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.viewport_sync.set_scale_factor(window.scale_factor());
        self.is_fullscreen = window.is_fullscreen();
        self.ensure_focus_subscriptions(window, cx);

        // 非连接状态下无 Canvas paint 阶段，仅当确实存在待回收纹理时执行一次清理
        if !matches!(self.state, RdpViewState::Connected { .. }) {
            self.frame_cache.borrow_mut().drain_stale_images(window);
        }

        let root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .bg(p.background)
            .overflow_hidden()
            .on_modifiers_changed(cx.listener(|this, e, _, _| this.handle_modifiers_changed(e)))
            .on_key_down(cx.listener(|this, e, _, cx| this.handle_key_down(e, cx)))
            .on_key_up(cx.listener(|this, e, _, _| this.handle_key_up(e)));

        match &self.state {
            RdpViewState::Connected { .. } => root
                .child(self.render_interactive_canvas(cx))
                .child(self.render_floating_toolbar(&p, cx)),
            _ => root.child(self.render_status_view(&p, cx)),
        }
    }
}
