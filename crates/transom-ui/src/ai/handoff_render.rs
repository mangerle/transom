//! 终端手动接管等待横幅渲染。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染终端手动接管等待横幅。
pub(super) fn render_handoff_banner(
    host_label: &str,
    command: &str,
    p: &Palette,
    on_continue: impl Fn(&mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    v_flex()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.primary)
        .child(
            h_flex()
                .gap(px(6.0))
                .items_center()
                .child(
                    div()
                        .text_color(p.primary)
                        .child(AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL)).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(rems(typography::CAPTION))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(
                            rust_i18n::t!("ai.handoff_banner_title", host = host_label).to_string(),
                        ),
                ),
        )
        .child(
            div()
                .p(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(p.background)
                .font_family("monospace")
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(command.to_owned()),
        )
        .child(
            h_flex()
                .justify_end()
                .gap(rems(space::XS))
                .child(
                    Button::new("ai-handoff-cancel")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(move |_, w, cx| on_cancel(w, cx)),
                )
                .child(
                    Button::new("ai-handoff-continue")
                        .primary()
                        .xsmall()
                        .label(rust_i18n::t!("ai.handoff_continue").to_string())
                        .on_click(move |_, w, cx| on_continue(w, cx)),
                ),
        )
        .into_any_element()
}
