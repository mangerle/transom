//! AI 输入卡片顶部的目标会话选择器胶囊渲染。

use gpui_kit::base::h_flex;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::app::ai::state::{ActiveAiSessionInfo, AiTargetMode};
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染输入卡片顶部的「目标会话选择器」胶囊。
pub(super) fn render_target_selector_bar(
    target_mode: AiTargetMode,
    active_session: Option<&ActiveAiSessionInfo>,
    popup_open: bool,
    p: &Palette,
    on_toggle_popup: impl Fn(&mut Window, &mut App) + 'static,
    on_unpin: super::ActionCb,
) -> AnyElement {
    let is_pinned = matches!(target_mode, AiTargetMode::PinnedTab(_)) && active_session.is_some();
    let label = match (target_mode, active_session) {
        (AiTargetMode::PinnedTab(_), Some(s)) => {
            rust_i18n::t!("ai.target_pinned_badge", host = s.host_name.as_str()).to_string()
        }
        (AiTargetMode::GlobalOnly, _) => rust_i18n::t!("ai.target_global_badge").to_string(),
        (_, Some(s)) => {
            rust_i18n::t!("ai.target_follow_badge", host = s.host_name.as_str()).to_string()
        }
        (_, None) => rust_i18n::t!("ai.target_follow_global").to_string(),
    };

    let leading: AnyElement = if is_pinned {
        div()
            .flex_shrink_0()
            .text_color(p.primary)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::LOCKED)).small())
            .into_any_element()
    } else if let Some(s) = active_session {
        let dot_color = if s.connected { p.success } else { p.text.muted };
        div()
            .w(px(6.0))
            .h(px(6.0))
            .flex_shrink_0()
            .rounded_full()
            .bg(dot_color)
            .into_any_element()
    } else {
        div()
            .w(px(6.0))
            .h(px(6.0))
            .flex_shrink_0()
            .rounded_full()
            .bg(p.text.muted)
            .into_any_element()
    };

    let mut selector = h_flex()
        .id("ai-target-session-btn")
        .min_w_0()
        .px(px(8.0))
        .py(px(2.0))
        .gap(px(5.0))
        .items_center()
        .rounded_full()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, w, cx| on_toggle_popup(w, cx));

    selector = if popup_open {
        selector.bg(p.surface.selected)
    } else if is_pinned {
        selector
            .bg(p.tone.primary_subtle)
            .hover(|s| s.bg(p.primary.opacity(0.18)))
    } else {
        selector.bg(p.secondary).hover(|s| s.bg(p.surface.hover))
    };

    selector = selector
        .child(leading)
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rems(typography::CAPTION))
                .font_weight(if is_pinned || popup_open {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if is_pinned { p.primary } else { p.foreground })
                .child(label),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(p.text.muted)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small()),
        );

    let mut left_group = h_flex()
        .flex_1()
        .min_w_0()
        .gap(px(4.0))
        .items_center()
        .child(selector);
    if is_pinned {
        let unpin_tip = rust_i18n::t!("ai.target_unpin_tip").to_string();
        left_group = left_group.child(
            h_flex()
                .id("ai-target-unpin-btn")
                .w(px(18.0))
                .h(px(18.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .text_color(p.text.muted)
                .hover(|s| s.bg(p.surface.hover).text_color(p.foreground))
                .tooltip(move |w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(unpin_tip.clone()).build(w, cx)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, w, cx| {
                    cx.stop_propagation();
                    on_unpin(w, cx);
                })
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small()),
        );
    }

    h_flex()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .items_center()
        .gap(rems(space::XS))
        .border_b_1()
        .border_color(p.divider(p.border))
        .child(left_group)
        .into_any_element()
}
