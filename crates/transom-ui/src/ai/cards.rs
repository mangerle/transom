//! AI 消息流气泡、可折叠思考块与工具结果卡片渲染。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::markdown_blocks::{CodeBlockCallbacks, MarkdownRenderParams, render_markdown_content};
use super::state::UiChatItem;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 单条消息项渲染参数。
pub(super) struct ChatItemRenderParams<'a> {
    pub(super) item: &'a UiChatItem,
    pub(super) expanded: bool,
    pub(super) copied_key: Option<&'a str>,
    pub(super) has_active_terminal: bool,
    pub(super) p: &'a Palette,
}

/// 卡片展开/收起回调类型。
pub(super) type ToggleCardCallback = Rc<dyn Fn(&mut Window, &mut App)>;
/// 回滚到指定轮次之前的回调类型。
pub(super) type RollbackCallback = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// 消息项交互回调集合。
#[derive(Clone)]
pub(super) struct ChatItemCallbacks {
    pub(super) on_toggle_expand: ToggleCardCallback,
    pub(super) on_rollback: RollbackCallback,
    pub(super) code: CodeBlockCallbacks,
}

/// 渲染单条消息项。
pub(super) fn render_chat_item(
    params: ChatItemRenderParams<'_>,
    callbacks: ChatItemCallbacks,
) -> AnyElement {
    let p = params.p;
    match params.item {
        UiChatItem::UserText { id, text } => render_user_bubble(id, text, &params, &callbacks),
        UiChatItem::Reasoning {
            id,
            text,
            streaming,
        } => render_reasoning_block(id, text, *streaming, params.expanded, p, callbacks),
        UiChatItem::AssistantText {
            id,
            text,
            model,
            tokens_per_sec,
            streaming,
        } => render_assistant_message(
            id,
            text,
            model.as_deref(),
            *tokens_per_sec,
            *streaming,
            &params,
            &callbacks,
        ),
        UiChatItem::ToolCard { .. } => super::tool_card::render_tool_card(&params, callbacks),
    }
}

fn render_user_bubble(
    id: &str,
    text: &str,
    params: &ChatItemRenderParams<'_>,
    callbacks: &ChatItemCallbacks,
) -> AnyElement {
    let p = params.p;
    let copy_key = format!("user-{id}");
    let is_copied = params.copied_key == Some(copy_key.as_str());
    let copy_icon = if is_copied {
        UiIcon::CHECK
    } else {
        UiIcon::COPY
    };
    let on_copy = Rc::clone(&callbacks.code.on_copy);
    let copied_text = text.to_owned();

    v_flex()
        .w_full()
        .items_end()
        .gap(px(4.0))
        .child(
            div()
                .max_w(rems(28.0))
                .px(rems(space::SM))
                .py(rems(space::XS))
                .rounded(p.radius.md)
                .bg(p.secondary)
                .border_1()
                .border_color(p.divider(p.border))
                .text_size(rems(typography::BODY))
                .text_color(p.foreground)
                .child(text.to_owned()),
        )
        .child(
            h_flex()
                .justify_end()
                .gap(px(6.0))
                .child({
                    let entry_id = id.to_owned();
                    let on_rb = Rc::clone(&callbacks.on_rollback);
                    Button::new(SharedString::from(format!("ai-rb-user-{id}")))
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("ai.rollback_turn").to_string())
                        .on_click(move |_, w, cx| on_rb(entry_id.clone(), w, cx))
                })
                .child(
                    Button::new(SharedString::from(format!("ai-copy-user-{id}")))
                        .ghost()
                        .xsmall()
                        .child(
                            h_flex()
                                .gap(px(4.0))
                                .items_center()
                                .child(AppIconElement::new(AppIcon::Ui(copy_icon)).small())
                                .child(if is_copied {
                                    rust_i18n::t!("ai.copied").to_string()
                                } else {
                                    rust_i18n::t!("ai.copy").to_string()
                                }),
                        )
                        .on_click(move |_, w, cx| {
                            on_copy(copy_key.clone(), copied_text.clone(), w, cx)
                        }),
                ),
        )
        .into_any_element()
}

fn render_reasoning_block(
    id: &str,
    text: &str,
    streaming: bool,
    expanded: bool,
    p: &Palette,
    callbacks: ChatItemCallbacks,
) -> AnyElement {
    let label = if streaming {
        rust_i18n::t!("ai.thinking").to_string()
    } else if expanded {
        rust_i18n::t!("ai.hide_reasoning").to_string()
    } else {
        rust_i18n::t!("ai.show_reasoning").to_string()
    };
    let chevron = if streaming || expanded {
        UiIcon::CHEVRON_DOWN
    } else {
        UiIcon::CHEVRON_RIGHT
    };
    let on_toggle = callbacks.on_toggle_expand;
    let mut box_el = v_flex()
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .rounded(p.radius.sm)
        .bg(p.secondary)
        .border_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .id(SharedString::from(format!("ai-reason-{id}")))
                .gap(px(4.0))
                .items_center()
                .cursor_pointer()
                .on_click(move |_, window, cx| on_toggle(window, cx))
                .child(
                    div()
                        .text_color(p.text.muted)
                        .child(AppIconElement::new(AppIcon::Ui(chevron)).small()),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(label),
                ),
        );
    if streaming || expanded {
        box_el = box_el.child(
            div()
                .mt(rems(space::XS))
                .pl(rems(space::SM))
                .border_l_2()
                .border_color(p.divider(p.border))
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(text.to_owned()),
        );
    }
    box_el.into_any_element()
}

fn render_assistant_message(
    id: &str,
    text: &str,
    model: Option<&str>,
    tokens_per_sec: Option<f32>,
    streaming: bool,
    params: &ChatItemRenderParams<'_>,
    callbacks: &ChatItemCallbacks,
) -> AnyElement {
    let p = params.p;
    let mut col = v_flex()
        .w_full()
        .py(rems(space::XS))
        .gap(rems(space::XS))
        .child(render_markdown_content(MarkdownRenderParams {
            msg_id: id,
            text,
            copied_key: params.copied_key,
            has_active_terminal: params.has_active_terminal,
            p,
            callbacks: &callbacks.code,
        }));

    if !streaming {
        let mut footer_parts = Vec::new();
        if let Some(m) = model {
            footer_parts.push(m.to_owned());
        }
        if let Some(tps) = tokens_per_sec {
            footer_parts.push(format!("{tps:.1} tok/s"));
        }
        let copy_key = format!("asst-{id}");
        let is_copied = params.copied_key == Some(copy_key.as_str());
        let copy_icon = if is_copied {
            UiIcon::CHECK
        } else {
            UiIcon::COPY
        };
        let on_copy = Rc::clone(&callbacks.code.on_copy);
        let full_text = text.to_owned();

        col = col.child(
            h_flex()
                .w_full()
                .pt(px(2.0))
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(footer_parts.join(" · ")),
                )
                .child(
                    Button::new(SharedString::from(format!("ai-copy-asst-{id}")))
                        .ghost()
                        .xsmall()
                        .child(
                            h_flex()
                                .gap(px(4.0))
                                .items_center()
                                .child(AppIconElement::new(AppIcon::Ui(copy_icon)).small())
                                .child(if is_copied {
                                    rust_i18n::t!("ai.copied").to_string()
                                } else {
                                    rust_i18n::t!("ai.copy").to_string()
                                }),
                        )
                        .on_click(move |_, w, cx| {
                            on_copy(copy_key.clone(), full_text.clone(), w, cx);
                        }),
                ),
        );
    }
    col.into_any_element()
}
