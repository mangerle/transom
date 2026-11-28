//! AI 面板底部下拉菜单：后端/Agent 切换、执行模式与模型/思考强度选择器。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai_contracts::{
    AcpAgentConfig, AiSettings, ExecutionMode, ThinkingEffort, default_acp_presets,
    is_acp_command_available,
};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 根据当前选中的后端/ACP Agent 返回专属图标。
pub(super) fn backend_icon(agent_id: Option<&str>) -> AppIcon {
    match agent_id {
        None => AppIcon::Custom(CustomIcon::AI_SPARKLE),
        Some(id) if id.contains("claude") => AppIcon::Custom(CustomIcon::ACP_CLAUDE),
        Some(id) if id.contains("codex") => AppIcon::Custom(CustomIcon::ACP_CODEX),
        Some(id) if id.contains("kimi") => AppIcon::Custom(CustomIcon::ACP_KIMI),
        Some(_) => AppIcon::Ui(UiIcon::RUN_ANYTHING),
    }
}

fn popup_card_shell(p: &Palette) -> Div {
    v_flex()
        .w_full()
        .p(rems(space::XXS))
        .gap(px(2.0))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .shadow_md()
}

/// 渲染后端 / 外部 ACP Agent 选择弹层。
pub(super) fn render_backend_popup(
    settings: &AiSettings,
    active_acp_id: Option<&str>,
    p: &Palette,
    on_select: impl Fn(Option<String>, &mut Window, &mut App) + Clone + 'static,
    on_open_config: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut agents: Vec<AcpAgentConfig> = settings.acp_agents.clone();
    for preset in default_acp_presets() {
        if !agents.iter().any(|a| a.id == preset.id) {
            agents.push(preset);
        }
    }

    let mut card = popup_card_shell(p);
    let builtin_active = active_acp_id.is_none();
    let pick_builtin = on_select.clone();
    card = card.child(render_backend_row(
        "ai-backend-builtin",
        rust_i18n::t!("ai.acp_builtin_mode").to_string(),
        backend_icon(None),
        builtin_active,
        true,
        p,
        move |w, cx| pick_builtin(None, w, cx),
    ));

    for (idx, agent) in agents.into_iter().enumerate() {
        let is_active = active_acp_id == Some(agent.id.as_str());
        let available = is_acp_command_available(&agent.command);
        let id = agent.id.clone();
        let icon = backend_icon(Some(&id));
        let pick = on_select.clone();
        card = card.child(render_backend_row(
            ("ai-backend-acp", idx),
            agent.name,
            icon,
            is_active,
            available,
            p,
            move |w, cx| pick(Some(id.clone()), w, cx),
        ));
    }

    card = card
        .child(
            div()
                .w_full()
                .h(px(1.0))
                .my(px(2.0))
                .bg(p.divider(p.border)),
        )
        .child(
            h_flex()
                .id("ai-backend-open-cfg")
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .gap(rems(space::XS))
                .items_center()
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, w, cx| on_open_config(w, cx))
                .child(
                    div()
                        .text_color(p.text.muted)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::SETTINGS)).small()),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("ai.popup_cli_manage").to_string()),
                ),
        );

    card.into_any_element()
}

fn render_backend_row(
    id: impl Into<ElementId>,
    label: String,
    icon: AppIcon,
    active: bool,
    available: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut row = h_flex()
        .id(id)
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .justify_between()
        .items_center()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, w, cx| on_click(w, cx));
    if active {
        row = row.bg(p.surface.selected);
    }
    row = row.child(
        h_flex()
            .gap(rems(space::XS))
            .items_center()
            .child(
                div()
                    .text_color(if active { p.primary } else { p.text.muted })
                    .child(AppIconElement::new(icon).small()),
            )
            .child(
                div()
                    .text_size(rems(typography::BODY))
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(p.foreground)
                    .child(label),
            ),
    );
    if active {
        row = row.child(
            div()
                .text_color(p.success)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
        );
    } else if !available {
        row = row.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.warning)
                .child(rust_i18n::t!("ai.acp_not_installed").to_string()),
        );
    }
    row.into_any_element()
}

/// 渲染执行模式选择弹层。
pub(super) fn render_mode_popup(
    current: ExecutionMode,
    p: &Palette,
    on_select: impl Fn(ExecutionMode, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let modes = [
        (
            ExecutionMode::Manual,
            AppIcon::Ui(UiIcon::LOCKED),
            rust_i18n::t!("ai.mode_manual_desc").to_string(),
        ),
        (
            ExecutionMode::SafeReadAuto,
            AppIcon::Ui(UiIcon::SHOW),
            rust_i18n::t!("ai.mode_safe_read_desc").to_string(),
        ),
        (
            ExecutionMode::Auto,
            AppIcon::Ui(UiIcon::LIGHTNING),
            rust_i18n::t!("ai.mode_auto_desc").to_string(),
        ),
        (
            ExecutionMode::AllowAll,
            AppIcon::Custom(CustomIcon::SHIELD_CHECK),
            rust_i18n::t!("ai.mode_allow_all_desc").to_string(),
        ),
    ];

    let mut card = popup_card_shell(p);
    for (idx, (mode, icon, desc)) in modes.into_iter().enumerate() {
        let active = mode == current;
        let pick = on_select.clone();
        let mut row = h_flex()
            .id(("ai-mode-opt", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::SM))
            .justify_between()
            .items_center()
            .rounded(p.radius.sm)
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| pick(mode, w, cx));
        if active {
            row = row.bg(p.surface.selected);
        }
        row = row.child(
            h_flex()
                .flex_1()
                .gap(rems(space::XS))
                .items_start()
                .child(
                    div()
                        .pt(px(2.0))
                        .text_color(if active { p.primary } else { p.text.muted })
                        .child(AppIconElement::new(icon).small()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(rems(typography::BODY))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .child(mode.label()),
                        )
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(desc),
                        ),
                ),
        );
        if active {
            row = row.child(
                div()
                    .text_color(p.success)
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
            );
        }
        card = card.child(row);
    }
    card.into_any_element()
}

/// 模型选择弹层参数。
pub(super) struct ModelPopupParams<'a> {
    pub(super) settings: &'a AiSettings,
    pub(super) active_acp: Option<&'a AcpAgentConfig>,
    pub(super) selected_model_id: &'a str,
    pub(super) selected_effort: Option<ThinkingEffort>,
    pub(super) p: &'a Palette,
}

/// 渲染模型与思考强度选择弹层（支持内置 AI 模型与外部 ACP Agent 模型切换）。
pub(super) fn render_model_popup(
    params: ModelPopupParams<'_>,
    on_select_model: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
    on_select_effort: impl Fn(Option<ThinkingEffort>, &mut Window, &mut App) + Clone + 'static,
    on_open_config: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let ModelPopupParams {
        settings,
        active_acp,
        selected_model_id,
        selected_effort,
        p,
    } = params;
    let mut card = popup_card_shell(p);
    if let Some(agent) = active_acp {
        let mut choices = agent.available_models.clone();
        if choices.is_empty()
            && let Some(preset) = default_acp_presets().into_iter().find(|p| p.id == agent.id)
        {
            choices = preset.available_models;
        }
        if !agent.default_model.trim().is_empty() && !choices.contains(&agent.default_model) {
            choices.insert(0, agent.default_model.clone());
        }
        let current_acp_model = if !agent.default_model.trim().is_empty() {
            agent.default_model.clone()
        } else {
            choices.first().cloned().unwrap_or_default()
        };
        let mut list = v_flex().w_full().max_h(px(200.0)).overflow_y_scrollbar();
        for (idx, model_name) in choices.into_iter().enumerate() {
            let active = model_name == current_acp_model;
            let pick = on_select_model.clone();
            let id_val = model_name.clone();
            let mut row = h_flex()
                .id(("ai-acp-model-opt", idx))
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .justify_between()
                .items_center()
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, w, cx| pick(id_val.clone(), w, cx));
            if active {
                row = row.bg(p.surface.selected);
            }
            row = row.child(
                v_flex()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(rems(typography::BODY))
                            .font_weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(p.foreground)
                            .child(model_name),
                    )
                    .child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(agent.name.clone()),
                    ),
            );
            if active {
                row = row.child(
                    div()
                        .text_color(p.success)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
                );
            }
            list = list.child(row);
        }
        return card.child(list).into_any_element();
    }

    let models: Vec<_> = settings.models().collect();
    if models.is_empty() {
        card = card.child(
            h_flex()
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(rust_i18n::t!("ai.no_model").to_string()),
                )
                .child(
                    Button::new("ai-model-empty-cfg")
                        .ghost()
                        .label(rust_i18n::t!("ai.acp_go_configure").to_string())
                        .on_click(move |_, w, cx| on_open_config(w, cx)),
                ),
        );
    } else {
        let mut list = v_flex().w_full().max_h(px(200.0)).overflow_y_scrollbar();
        for (idx, (provider, model)) in models.into_iter().enumerate() {
            let active = model.id == selected_model_id;
            let model_id = model.id.clone();
            let pick = on_select_model.clone();
            let mut row = h_flex()
                .id(("ai-model-opt", idx))
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .justify_between()
                .items_center()
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, w, cx| pick(model_id.clone(), w, cx));
            if active {
                row = row.bg(p.surface.selected);
            }
            row = row.child(
                v_flex()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(rems(typography::BODY))
                            .font_weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(p.foreground)
                            .child(model.display_label().to_owned()),
                    )
                    .child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(if model.display_name.trim().is_empty() {
                                provider.name.clone()
                            } else {
                                format!("{} · {}", provider.name, model.name)
                            }),
                    ),
            );
            if active {
                row = row.child(
                    div()
                        .text_color(p.success)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
                );
            }
            list = list.child(row);
        }
        card = card.child(list);
    }

    let efforts = [
        (None, rust_i18n::t!("ai.effort_default").to_string()),
        (Some(ThinkingEffort::Low), "low".to_owned()),
        (Some(ThinkingEffort::Medium), "medium".to_owned()),
        (Some(ThinkingEffort::High), "high".to_owned()),
    ];
    let mut effort_bar = v_flex()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(px(6.0))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child(rust_i18n::t!("ai.popup_thinking_effort").to_string()),
        );
    let mut btns = h_flex().w_full().gap(px(4.0));
    for (idx, (eff, label)) in efforts.into_iter().enumerate() {
        let pick_eff = on_select_effort.clone();
        let active = selected_effort == eff;
        let mut opt = h_flex()
            .id(("ai-effort-opt", idx))
            .flex_1()
            .min_w_0()
            .py(px(4.0))
            .items_center()
            .justify_center()
            .rounded(p.radius.sm)
            .cursor_pointer()
            .text_size(rems(typography::CAPTION))
            .on_click(move |_, w, cx| pick_eff(eff, w, cx))
            .child(div().truncate().child(label));
        opt = if active {
            opt.bg(p.border.opacity(0.42))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
        } else {
            opt.text_color(p.text.muted)
                .hover(|s| s.bg(p.surface.hover).text_color(p.foreground))
        };
        btns = btns.child(opt);
    }
    effort_bar = effort_bar.child(btns);

    card.child(
        div()
            .w_full()
            .h(px(1.0))
            .my(px(2.0))
            .bg(p.divider(p.border)),
    )
    .child(effort_bar)
    .into_any_element()
}
