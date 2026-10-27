//! 会话表单中出站代理（Direct, SOCKS5, HTTP, FollowGlobal）配置渲染模块。

use super::types::{ProxyChoice, SessionForm};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;

/// 渲染出站代理配置分栏（跟随全局、直连或会话专属 SOCKS5/HTTP）。
pub(crate) fn render_proxy_section(
    form: &SessionForm,
    cx: &mut Context<AppShell>,
    p: &Palette,
) -> AnyElement {
    let proxy_choice = form.proxy_choice;
    let global_proxy = form.global_proxy.describe();
    let proxy_host = form.proxy_host.clone();
    let proxy_port = form.proxy_port.clone();
    let proxy_username = form.proxy_username.clone();
    let proxy_password = form.proxy_password.clone();
    let mut proxy_section = v_flex().gap_2().child(
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("form.proxy").to_string()),
    );
    proxy_section = proxy_section.child(
        h_flex().gap_2().children(
            [
                (
                    "proxy-follow-global",
                    "form.proxy_follow_global",
                    ProxyChoice::FollowGlobal,
                ),
                ("proxy-direct", "form.proxy_direct", ProxyChoice::Direct),
                ("proxy-socks5", "form.proxy_socks5", ProxyChoice::Socks5),
                ("proxy-http", "form.proxy_http", ProxyChoice::Http),
            ]
            .into_iter()
            .map(|(id, key, choice)| {
                Button::new(id)
                    .small()
                    .label(rust_i18n::t!(key).to_string())
                    .selected(proxy_choice == choice)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(form) = this.modals.form.as_mut() {
                            form.proxy_choice = choice;
                        }
                        cx.notify();
                    }))
            }),
        ),
    );
    if proxy_choice == ProxyChoice::FollowGlobal {
        proxy_section = proxy_section.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.proxy_global_hint", proxy = global_proxy).to_string()),
        );
    } else if proxy_choice != ProxyChoice::Direct {
        proxy_section = proxy_section
            .child(
                h_flex()
                    .gap_3()
                    .items_end()
                    .child(div().flex_1().child(AppShell::form_field(
                        rust_i18n::t!("form.proxy_host").to_string(),
                        proxy_host,
                        p,
                    )))
                    .child(div().w(px(110.0)).child(AppShell::form_field(
                        rust_i18n::t!("form.proxy_port").to_string(),
                        proxy_port,
                        p,
                    ))),
            )
            .child(AppShell::form_field(
                rust_i18n::t!("form.proxy_username").to_string(),
                proxy_username,
                p,
            ))
            .child(AppShell::secret_field(
                rust_i18n::t!("form.proxy_password").to_string(),
                proxy_password,
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.proxy_auth_hint").to_string()),
            );
    }
    proxy_section.into_any_element()
}
