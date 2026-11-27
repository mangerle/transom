//! AI 面板底部提问卡片：ACP 缺失告警条、附件卡片、多行输入区与单行控制条。

use std::path::PathBuf;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InlineTokenContext, Textarea, TextareaState};
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::gpui::{ClipboardItem, ExternalPaths};
use gpui_kit::*;
use transom_ai::ExecutionMode;
use transom_core::TabId;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use super::attachments::{ComposerAttachment, render_attachment_chips};
use super::composer_triggers::{
    mode_icon, render_icon_popup_trigger, render_metric_chip, render_popup_trigger,
};
use super::metrics::{AiSessionMetrics, anchor_toolbar_popover, format_compact_tokens};
use super::slash_menu::{AcpBannerParams, render_acp_warning_banner};
use super::state::AiActivePopup;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 可加入多主机协同 Scope 的主机选项。
pub(super) struct ExtraHostOption {
    pub(super) tab_id: TabId,
    pub(super) host_name: String,
    pub(super) selected: bool,
}

/// 渲染输入区构造参数。
pub(super) struct ComposerParams<'a> {
    pub(super) input: &'a Entity<TextareaState>,
    pub(super) is_running: bool,
    pub(super) model_label: String,
    pub(super) execution_mode: ExecutionMode,
    pub(super) session_badge: Option<String>,
    pub(super) acp_label: String,
    pub(super) active_acp_id: Option<String>,
    pub(super) acp_missing_warning: Option<(String, String, String)>,
    pub(super) acp_install_status: Option<&'a (String, bool, String)>,
    pub(super) active_popup: AiActivePopup,
    pub(super) toolbar_popover: Option<(AiActivePopup, AnyElement)>,
    pub(super) pending_attachments: &'a [ComposerAttachment],
    pub(super) metrics: &'a AiSessionMetrics,
    pub(super) context_window: u32,
    pub(super) extra_host_options: Vec<ExtraHostOption>,
    pub(super) status_header: Option<AnyElement>,
    pub(super) p: &'a Palette,
}

type ActionCb = Box<dyn Fn(&mut Window, &mut App)>;
type PopupCb = Box<dyn Fn(AiActivePopup, &mut Window, &mut App)>;
type TabCb = Box<dyn Fn(TabId, &mut Window, &mut App)>;
type IdCb = Box<dyn Fn(String, &mut Window, &mut App)>;
type PasteCb = Box<dyn Fn(&ClipboardItem, &mut Window, &mut App) -> bool>;
type DropCb = Box<dyn Fn(Vec<PathBuf>, &mut Window, &mut App)>;

/// 底部输入卡片的交互回调集合。
pub(super) struct ComposerActions {
    pub(super) on_send: ActionCb,
    pub(super) on_stop: ActionCb,
    pub(super) on_toggle_popup: PopupCb,
    pub(super) on_toggle_extra_host: TabCb,
    pub(super) on_switch_builtin: ActionCb,
    pub(super) on_open_config: ActionCb,
    pub(super) on_install_acp: IdCb,
    pub(super) on_remove_attachment: IdCb,
    pub(super) on_paste_item: PasteCb,
    pub(super) on_drop_paths: DropCb,
}

/// 渲染底部 CC-GUI 风格圆角提问卡片。
pub(super) fn render_composer(params: ComposerParams<'_>, actions: ComposerActions) -> AnyElement {
    let p = params.p;
    let ComposerActions {
        on_send,
        on_stop,
        on_toggle_popup,
        on_toggle_extra_host,
        on_switch_builtin,
        on_open_config,
        on_install_acp,
        on_remove_attachment,
        on_paste_item,
        on_drop_paths,
    } = actions;
    let toggle_popup_rc = std::rc::Rc::new(on_toggle_popup);
    let toggle_host_rc = std::rc::Rc::new(on_toggle_extra_host);

    let drag_border = p.primary;
    let mut card = v_flex()
        .w_full()
        .rounded(p.radius.lg)
        .border_1()
        .border_color(p.divider(p.border))
        .bg(p.surface.floating)
        .shadow_sm()
        .overflow_hidden()
        .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(drag_border))
        .on_drop::<ExternalPaths>(move |ext, w, cx| {
            on_drop_paths(ext.paths().to_vec(), w, cx);
        });

    if let Some(status_header) = params.status_header {
        card = card.child(status_header);
    }

    if let Some((agent_id, agent, cmd)) = params.acp_missing_warning {
        let (installing, detail) = match params.acp_install_status {
            Some((id, running, msg)) if id == &agent_id => (*running, Some(msg.as_str())),
            _ => (false, None),
        };
        card = card.child(render_acp_warning_banner(
            AcpBannerParams {
                agent_id: &agent_id,
                agent: &agent,
                cmd: &cmd,
                installing,
                install_detail: detail,
                p,
            },
            on_install_acp,
            on_switch_builtin,
            on_open_config,
        ));
    }

    if let Some(badge) = params.session_badge {
        let dot_color = if params.extra_host_options.is_empty() && !badge.contains('@') {
            p.text.muted
        } else {
            p.success
        };
        card = card.child(
            h_flex()
                .w_full()
                .px(rems(space::SM))
                .pt(rems(space::XS))
                .items_center()
                .gap(rems(space::XS))
                .child(
                    h_flex()
                        .px(px(8.0))
                        .py(px(2.0))
                        .gap(px(5.0))
                        .items_center()
                        .rounded_full()
                        .bg(p.secondary)
                        .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(dot_color))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(badge),
                        ),
                ),
        );
    }

    if !params.pending_attachments.is_empty() {
        let rm_rc = std::rc::Rc::new(on_remove_attachment);
        card = card.child(render_attachment_chips(
            params.pending_attachments,
            p,
            move |id, w, cx| rm_rc(id, w, cx),
        ));
    }

    if !params.extra_host_options.is_empty() {
        let mut scope_row = h_flex()
            .w_full()
            .px(rems(space::SM))
            .pt(rems(space::XXS))
            .gap(rems(space::XXS))
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(rust_i18n::t!("ai.bulk_scope_label").to_string()),
            );
        for (idx, opt) in params.extra_host_options.into_iter().enumerate() {
            let toggle = toggle_host_rc.clone();
            let tid = opt.tab_id;
            let btn = Button::new(("ai-extra-host", idx)).label(opt.host_name);
            let btn = if opt.selected {
                btn.primary()
            } else {
                btn.ghost()
            };
            scope_row = scope_row.child(btn.on_click(move |_, w, cx| toggle(tid, w, cx)));
        }
        card = card.child(scope_row);
    }

    let t_backend = toggle_popup_rc.clone();
    let t_mode = toggle_popup_rc.clone();
    let t_model = toggle_popup_rc.clone();
    let t_stats = toggle_popup_rc.clone();
    let t_usage = toggle_popup_rc.clone();
    let t_ctx = toggle_popup_rc;

    let tps_label = params
        .metrics
        .last_tps
        .map(|t| format!("{t:.0} tok/s"))
        .unwrap_or_else(|| "0 tok/s".to_owned());
    let usage_label = rust_i18n::t!(
        "ai.usage_chip_cache",
        total = format_compact_tokens(params.metrics.total_tokens()),
        hit = params.metrics.cache_hit_percent()
    )
    .to_string();
    let ctx_pct = params.metrics.context_percent(params.context_window);
    let backend_trigger_icon = super::popups::backend_icon(params.active_acp_id.as_deref());

    let left_controls = h_flex()
        .flex_1()
        .overflow_hidden()
        .gap(px(2.0))
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(render_icon_popup_trigger(
            "ai-popup-backend",
            backend_trigger_icon,
            params.acp_label,
            params.active_popup == AiActivePopup::Backend,
            p,
            move |w, cx| t_backend(AiActivePopup::Backend, w, cx),
        ))
        .child(render_icon_popup_trigger(
            "ai-popup-mode",
            mode_icon(params.execution_mode),
            params.execution_mode.label().to_owned(),
            params.active_popup == AiActivePopup::Mode,
            p,
            move |w, cx| t_mode(AiActivePopup::Mode, w, cx),
        ))
        .child(render_popup_trigger(
            "ai-popup-model",
            AppIcon::Custom(CustomIcon::CPU),
            params.model_label,
            params.active_popup == AiActivePopup::Model,
            p,
            move |w, cx| t_model(AiActivePopup::Model, w, cx),
        ))
        .child(render_metric_chip(
            "ai-chip-stats",
            div()
                .text_color(if params.active_popup == AiActivePopup::SessionStats {
                    p.primary
                } else {
                    p.text.muted
                })
                .child(AppIconElement::new(AppIcon::Custom(CustomIcon::SPEEDOMETER)).small())
                .into_any_element(),
            tps_label,
            params.active_popup == AiActivePopup::SessionStats,
            p,
            move |w, cx| t_stats(AiActivePopup::SessionStats, w, cx),
        ))
        .child(render_metric_chip(
            "ai-chip-usage",
            div()
                .text_color(if params.active_popup == AiActivePopup::TokenUsage {
                    p.primary
                } else {
                    p.text.muted
                })
                .child(AppIconElement::new(AppIcon::Custom(CustomIcon::TOKENS)).small())
                .into_any_element(),
            usage_label,
            params.active_popup == AiActivePopup::TokenUsage,
            p,
            move |w, cx| t_usage(AiActivePopup::TokenUsage, w, cx),
        ))
        .child(render_metric_chip(
            "ai-chip-ctx",
            ProgressCircle::new("ai-ctx-ring")
                .value(ctx_pct as f32)
                .size(px(14.0))
                .color(if ctx_pct >= 80 {
                    p.danger
                } else if ctx_pct >= 60 {
                    p.warning
                } else {
                    p.foreground
                })
                .into_any_element(),
            format!("{ctx_pct}%"),
            params.active_popup == AiActivePopup::ContextUsage,
            p,
            move |w, cx| t_ctx(AiActivePopup::ContextUsage, w, cx),
        ));

    let send_tip = rust_i18n::t!("ai.send").to_string();
    let stop_tip = rust_i18n::t!("ai.stop").to_string();
    let action_btn = if params.is_running {
        h_flex()
            .id("ai-stop-btn")
            .w(px(28.0))
            .h(px(28.0))
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(p.secondary)
            .border_1()
            .border_color(p.divider(p.border))
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .tooltip(move |w, cx| {
                gpui_kit::component::tooltip::Tooltip::new(stop_tip.clone()).build(w, cx)
            })
            .on_click(move |_, w, cx| on_stop(w, cx))
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::STOP))
                    .small()
                    .monochrome(p.danger),
            )
            .into_any_element()
    } else {
        h_flex()
            .id("ai-send-btn")
            .w(px(28.0))
            .h(px(28.0))
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(p.primary)
            .cursor_pointer()
            .hover(|s| s.bg(p.primary.opacity(0.88)))
            .tooltip(move |w, cx| {
                gpui_kit::component::tooltip::Tooltip::new(send_tip.clone()).build(w, cx)
            })
            .on_click(move |_, w, cx| on_send(w, cx))
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::ARROW_RIGHT))
                    .small()
                    .monochrome(gpui_kit::white()),
            )
            .into_any_element()
    };

    let token_palette = p.clone();
    let input_box = div()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .bg(p.background)
        .child(
            Textarea::new(params.input)
                .appearance(false)
                .bordered(false)
                .h(rems(9.5))
                .token(move |token_ctx, _, _| render_inline_chip(token_ctx, &token_palette))
                .on_paste(move |item, w, cx| on_paste_item(item, w, cx)),
        );

    card = card
        .child(input_box)
        .child(
            h_flex()
                .w_full()
                .px(rems(space::XS))
                .py(rems(space::XS))
                .gap(rems(space::XXS))
                .justify_between()
                .items_center()
                .border_t_1()
                .border_color(p.divider(p.border))
                .bg(p.secondary)
                .child(left_controls)
                .child(div().flex_shrink_0().child(action_btn)),
        );

    let mut wrapper = div()
        .relative()
        .w_full()
        .px(rems(space::SM))
        .pb(rems(space::SM))
        .child(card);
    if let Some((kind, pop)) = params.toolbar_popover {
        wrapper = wrapper.child(anchor_toolbar_popover(kind, pop));
    }
    wrapper.into_any_element()
}

/// 渲染输入框内的原子行内指令/主机标签（与文本行高、光标同一行对齐）。
fn render_inline_chip(token_ctx: &InlineTokenContext, p: &Palette) -> AnyElement {
    let icon = if token_ctx.token().id().starts_with("slash:") {
        UiIcon::LIGHTNING
    } else {
        UiIcon::WEB_SERVER
    };
    let (bg, border) = if token_ctx.is_selected() {
        (p.primary.opacity(0.24), p.primary)
    } else {
        (p.primary.opacity(0.12), p.primary.opacity(0.32))
    };
    h_flex()
        .h(token_ctx.line_height())
        .max_w(token_ctx.available_width())
        .px(px(6.0))
        .gap(px(4.0))
        .items_center()
        .rounded(p.radius.sm)
        .bg(bg)
        .border_1()
        .border_color(border)
        .child(
            div()
                .flex_shrink_0()
                .text_color(p.primary)
                .child(AppIconElement::new(AppIcon::Ui(icon)).small()),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rems(typography::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.primary)
                .child(token_ctx.token().label().clone()),
        )
        .into_any_element()
}
