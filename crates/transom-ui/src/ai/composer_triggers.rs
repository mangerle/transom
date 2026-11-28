//! 底部提问卡片工具栏触发器与指标胶囊渲染辅助。

use gpui_kit::base::h_flex;
use gpui_kit::*;
use transom_ai_contracts::ExecutionMode;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::typography;

impl AiAssistantView {
    pub(super) fn active_context_window(&self) -> u32 {
        self.state
            .config
            .settings
            .resolve_model(&self.state.config.selected_model_id)
            .and_then(|(_, m)| m.context_window)
            .unwrap_or(128_000)
    }
}

pub(super) fn mode_icon(mode: ExecutionMode) -> AppIcon {
    match mode {
        ExecutionMode::Manual => AppIcon::Ui(UiIcon::LOCKED),
        ExecutionMode::SafeReadAuto => AppIcon::Ui(UiIcon::SHOW),
        ExecutionMode::Auto => AppIcon::Ui(UiIcon::LIGHTNING),
        ExecutionMode::AllowAll => AppIcon::Custom(CustomIcon::SHIELD_CHECK),
    }
}

pub(super) fn render_icon_popup_trigger(
    id: &'static str,
    icon: AppIcon,
    tooltip: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut btn = h_flex()
        .id(id)
        .px(px(6.0))
        .py(px(3.0))
        .gap(px(2.0))
        .items_center()
        .flex_shrink_0()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .tooltip(move |w, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(w, cx)
        })
        .on_click(move |_, w, cx| on_click(w, cx));
    if active {
        btn = btn.bg(p.surface.selected);
    }
    btn.child(
        div()
            .text_color(if active { p.primary } else { p.text.muted })
            .child(AppIconElement::new(icon).small()),
    )
    .child(
        div()
            .text_color(p.text.muted)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small()),
    )
    .into_any_element()
}

pub(super) fn render_popup_trigger(
    id: &'static str,
    icon: AppIcon,
    label: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut btn = h_flex()
        .id(id)
        .px(px(6.0))
        .py(px(3.0))
        .gap(px(4.0))
        .items_center()
        .flex_shrink_0()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, w, cx| on_click(w, cx));
    if active {
        btn = btn.bg(p.surface.selected);
    }
    btn.child(
        div()
            .text_color(if active { p.primary } else { p.text.muted })
            .child(AppIconElement::new(icon).small()),
    )
    .child(
        div()
            .max_w(px(110.0))
            .truncate()
            .text_size(rems(typography::CAPTION))
            .text_color(p.foreground)
            .child(label),
    )
    .child(
        div()
            .text_color(p.text.muted)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small()),
    )
    .into_any_element()
}

pub(super) fn render_metric_chip(
    id: &'static str,
    icon_el: AnyElement,
    label: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut chip = h_flex()
        .id(id)
        .px(px(5.0))
        .py(px(3.0))
        .gap(px(3.0))
        .items_center()
        .flex_shrink_0()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, w, cx| on_click(w, cx));
    if active {
        chip = chip.bg(p.surface.selected);
    }
    chip.child(icon_el)
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(if active { p.foreground } else { p.text.muted })
                .child(label),
        )
        .into_any_element()
}
