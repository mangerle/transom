//! 输入卡片顶部的 ACP CLI 未安装告警横幅渲染。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

pub(super) struct AcpBannerParams<'a> {
    pub(super) agent_id: &'a str,
    pub(super) agent: &'a str,
    pub(super) cmd: &'a str,
    pub(super) installing: bool,
    pub(super) install_detail: Option<&'a str>,
    pub(super) p: &'a Palette,
}

/// 渲染输入卡片顶部的 ACP CLI 未安装告警横幅。
pub(super) fn render_acp_warning_banner(
    params: AcpBannerParams<'_>,
    on_install: impl Fn(String, &mut Window, &mut App) + 'static,
    on_switch_builtin: impl Fn(&mut Window, &mut App) + 'static,
    on_open_config: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let AcpBannerParams {
        agent_id,
        agent,
        cmd,
        installing,
        install_detail,
        p,
    } = params;
    let text = rust_i18n::t!("ai.acp_missing_banner", agent = agent, cmd = cmd).to_string();
    let aid = agent_id.to_owned();
    let install_label = if installing {
        rust_i18n::t!("ai.acp_installing").to_string()
    } else {
        rust_i18n::t!("ai.acp_install_btn").to_string()
    };
    let mut banner = v_flex()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(px(4.0))
        .bg(p.tone.warning_subtle)
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .w_full()
                .gap(rems(space::XS))
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .flex_1()
                        .overflow_hidden()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(
                            div()
                                .text_color(p.warning)
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::WARNING)).small()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.warning)
                                .child(text),
                        ),
                )
                .child(
                    h_flex()
                        .gap(rems(space::XXS))
                        .flex_shrink_0()
                        .child(
                            Button::new("ai-acp-warn-builtin")
                                .ghost()
                                .label(rust_i18n::t!("ai.acp_switch_builtin").to_string())
                                .on_click(move |_, w, cx| on_switch_builtin(w, cx)),
                        )
                        .child(
                            Button::new("ai-acp-warn-cfg")
                                .ghost()
                                .label(rust_i18n::t!("ai.acp_go_configure").to_string())
                                .on_click(move |_, w, cx| on_open_config(w, cx)),
                        )
                        .child(
                            Button::new("ai-acp-warn-install")
                                .primary()
                                .label(install_label)
                                .on_click(move |_, w, cx| {
                                    if !installing {
                                        on_install(aid.clone(), w, cx);
                                    }
                                }),
                        ),
                ),
        );
    if let Some(detail) = install_detail {
        banner = banner.child(
            div()
                .w_full()
                .truncate()
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child(detail.to_owned()),
        );
    }
    banner.into_any_element()
}
