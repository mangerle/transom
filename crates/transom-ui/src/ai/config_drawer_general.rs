//! AI 配置抽屉“通用与 Agent（ACP）”分页渲染。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::select::Select;
use gpui_kit::*;
use transom_ai::{default_acp_presets, is_acp_command_available};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::config_drawer::{
    ConfigDrawerActions, ConfigDrawerParams, ConfigSelect, field_label, section_card,
};
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

fn render_preference_row(label: String, select: &Entity<ConfigSelect>, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .py(px(2.0))
        .gap(rems(space::SM))
        .justify_between()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.foreground)
                .child(label),
        )
        .child(
            div()
                .w(px(148.0))
                .flex_shrink_0()
                .child(Select::new(select).small().w_full()),
        )
        .into_any_element()
}

pub(super) fn render_general_acp_tab(
    params: ConfigDrawerParams<'_>,
    actions: ConfigDrawerActions,
) -> Vec<AnyElement> {
    let p = params.p;
    let pref_card = section_card(rust_i18n::t!("ai.pref_section_title").to_string(), None, p)
        .child(render_preference_row(
            rust_i18n::t!("ai.cfg_language_label").to_string(),
            params.lang_select,
            p,
        ))
        .child(
            div()
                .w_full()
                .border_t_1()
                .border_color(p.divider(p.border))
                .pt(rems(space::XS))
                .child(render_preference_row(
                    rust_i18n::t!("ai.running_msg_title").to_string(),
                    params.running_behavior_select,
                    p,
                )),
        );

    let acp_presets = default_acp_presets();
    let mut acp_card = section_card(rust_i18n::t!("ai.acp_section_title").to_string(), None, p);
    for (idx, preset) in acp_presets.iter().enumerate() {
        let selected = idx == params.selected_acp_idx;
        let saved_cmd = params
            .settings
            .acp_agents
            .iter()
            .find(|a| a.id == preset.id)
            .map(|a| a.command.as_str());
        let effective_cmd = saved_cmd.unwrap_or(&preset.command);
        let installed = is_acp_command_available(effective_cmd);
        let installing = params
            .acp_install_status
            .as_ref()
            .is_some_and(|(id, run, _)| id == &preset.id && *run);
        let cb_sel = actions.on_apply_acp_preset.clone();
        let cb_inst = actions.on_install_acp.clone();
        let pid = preset.id.clone();

        let status_el = if installed {
            h_flex()
                .flex_shrink_0()
                .px(px(8.0))
                .py(px(3.0))
                .gap(px(4.0))
                .items_center()
                .rounded(p.radius.sm)
                .bg(p.tone.success_subtle)
                .child(
                    div()
                        .text_color(p.success)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small()),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.success)
                        .child(rust_i18n::t!("ai.acp_installed").to_string()),
                )
                .into_any_element()
        } else {
            let lbl = if installing {
                rust_i18n::t!("ai.acp_installing").to_string()
            } else {
                rust_i18n::t!("ai.acp_download_install").to_string()
            };
            Button::new(("ai-acp-install", idx))
                .outline()
                .xsmall()
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .items_center()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                        .child(lbl),
                )
                .on_click(move |_, w, cx| {
                    cx.stop_propagation();
                    cb_inst(pid.clone(), w, cx);
                })
                .into_any_element()
        };

        let mut row = h_flex()
            .id(("ai-acp-row", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .justify_between()
            .items_center()
            .rounded(p.radius.sm)
            .border_1()
            .cursor_pointer()
            .on_click(move |_, w, cx| cb_sel(idx, w, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(rems(space::XS))
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if selected { p.primary } else { p.text.muted })
                            .child(
                                AppIconElement::new(super::popups::backend_icon(Some(&preset.id)))
                                    .small(),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::CAPTION))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.foreground)
                                    .child(preset.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family("monospace")
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(if installed {
                                        format!("$ {effective_cmd}")
                                    } else {
                                        preset.install_command.clone()
                                    }),
                            ),
                    ),
            )
            .child(status_el);
        row = if selected {
            row.bg(p.primary.opacity(0.08))
                .border_color(p.primary.opacity(0.45))
        } else {
            row.bg(p.secondary)
                .border_color(p.divider(p.border))
                .hover(|s| s.bg(p.surface.hover))
        };
        acp_card = acp_card.child(row);
    }

    acp_card = acp_card.child(
        v_flex()
            .w_full()
            .pt(rems(space::XS))
            .gap(px(4.0))
            .border_t_1()
            .border_color(p.divider(p.border))
            .child(field_label(
                rust_i18n::t!("ai.acp_cmd_label").to_string(),
                p,
            ))
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(params.acp_cmd_input).small()),
                    )
                    .child(
                        Button::new("ai-save-acp")
                            .primary()
                            .small()
                            .label(rust_i18n::t!("ai.acp_save").to_string())
                            .on_click(move |_, w, cx| (actions.on_save_acp)(w, cx)),
                    ),
            ),
    );
    if let Some((_, _, detail)) = params.acp_install_status {
        acp_card = acp_card.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.primary)
                .child(detail.clone()),
        );
    }

    vec![pref_card.into_any_element(), acp_card.into_any_element()]
}
