//! AI 助手空会话欢迎页与快捷建议卡片渲染。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, DomainIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染 AI 面板顶栏带激活高亮与 tooltip 的图标按钮。
pub(super) fn header_icon_btn(
    id: &'static str,
    icon: AppIcon,
    label: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let btn = Button::new(id)
        .ghost()
        .accessibility_label(label.clone())
        .tooltip(label)
        .child(AppIconElement::new(icon).small())
        .on_click(move |_, w, cx| on_click(w, cx));
    let mut wrap = div().rounded(p.radius.sm);
    if active {
        wrap = wrap.bg(p.surface.selected);
    }
    wrap.child(btn).into_any_element()
}

/// 渲染空会话时的居中欢迎引导页与快捷建议卡片。
pub(super) fn render_empty_state(
    active_agent_id: Option<&str>,
    active_agent_name: Option<&str>,
    p: &Palette,
    on_quick_prompt: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let greeting = match active_agent_name {
        Some(name) => rust_i18n::t!("ai.empty_greeting_acp", agent = name).to_string(),
        None => rust_i18n::t!("ai.empty_greeting").to_string(),
    };
    let icon = super::popups::backend_icon(active_agent_id);
    let quick_items = [
        (
            AppIcon::Custom(CustomIcon::SPEEDOMETER),
            rust_i18n::t!("ai.quick_inspect_title").to_string(),
            rust_i18n::t!("ai.quick_inspect_desc").to_string(),
            "/inspect".to_owned(),
        ),
        (
            AppIcon::Custom(CustomIcon::DATABASE),
            rust_i18n::t!("ai.quick_disk_title").to_string(),
            rust_i18n::t!("ai.quick_disk_desc").to_string(),
            "/disk".to_owned(),
        ),
        (
            AppIcon::Custom(CustomIcon::CPU),
            rust_i18n::t!("ai.quick_docker_title").to_string(),
            rust_i18n::t!("ai.quick_docker_desc").to_string(),
            "/docker".to_owned(),
        ),
        (
            AppIcon::Domain(DomainIcon::TERMINAL),
            rust_i18n::t!("ai.quick_sessions_title").to_string(),
            rust_i18n::t!("ai.quick_sessions_desc").to_string(),
            rust_i18n::t!("ai.quick_sessions_prompt").to_string(),
        ),
    ];

    let mut cards = v_flex().w_full().max_w(rems(26.0)).gap(rems(space::XS));
    for (idx, (q_icon, title, desc, prompt)) in quick_items.into_iter().enumerate() {
        let send = on_quick_prompt.clone();
        cards = cards.child(
            h_flex()
                .id(("ai-quick-card", idx))
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .gap(rems(space::SM))
                .items_center()
                .rounded(p.radius.md)
                .bg(p.secondary)
                .border_1()
                .border_color(p.divider(p.border))
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover).border_color(p.primary))
                .on_click(move |_, w, cx| send(prompt.clone(), w, cx))
                .child(
                    div()
                        .w(rems(1.8))
                        .h(rems(1.8))
                        .rounded(p.radius.sm)
                        .bg(p.background)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(p.primary)
                        .child(AppIconElement::new(q_icon).small()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .overflow_hidden()
                        .gap(px(2.0))
                        .child(
                            div()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(title),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(desc),
                        ),
                ),
        );
    }

    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .px(rems(space::LG))
        .gap(rems(space::MD))
        .child(
            div()
                .w(rems(3.2))
                .h(rems(3.2))
                .rounded(p.radius.lg)
                .bg(p.secondary)
                .border_1()
                .border_color(p.divider(p.border))
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.primary)
                .child(AppIconElement::new(icon).with_rems(rems(1.6))),
        )
        .child(
            v_flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(rems(typography::TITLE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .text_center()
                        .child(greeting),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .text_center()
                        .child(rust_i18n::t!("ai.empty_subhint").to_string()),
                ),
        )
        .child(cards)
        .into_any_element()
}
