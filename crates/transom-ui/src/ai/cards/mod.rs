//! AI 消息流气泡、可折叠思考块与工具结果卡片渲染。

pub(in crate::app::ai) mod approval;
mod assistant_card;
pub(in crate::app::ai) mod bulk_matrix;
pub(in crate::app::ai) mod choice_card;
pub(in crate::app::ai) mod diff_view;
pub(in crate::app::ai) mod plan_card;
pub(in crate::app::ai) mod queue_card;
pub(in crate::app::ai) mod tool_card;
pub(in crate::app::ai) mod tool_card_body;
pub(in crate::app::ai) mod work_group;
pub(in crate::app::ai) mod work_group_model;
pub(in crate::app::ai) mod work_group_summary;

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::app::ai::markdown_blocks::{CodeBlockCallbacks, StreamingMarkdownCache};
use crate::app::ai::state::UiChatItem;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use assistant_card::{AssistantMessageParams, render_assistant_message};

/// 单条消息项渲染参数。
pub(in crate::app::ai) struct ChatItemRenderParams<'a> {
    pub(in crate::app::ai) item: &'a UiChatItem,
    pub(in crate::app::ai) expanded: bool,
    pub(in crate::app::ai) full_output_expanded: bool,
    pub(in crate::app::ai) copied_key: Option<&'a str>,
    pub(in crate::app::ai) has_active_terminal: bool,
    /// 帧级统一时间戳，避免每个用户气泡重复调用 `Utc::now()`。
    pub(in crate::app::ai) now: chrono::DateTime<chrono::Utc>,
    /// 若当前项是 `delegate_task`，则关联对应的实时子代理托管任务状态。
    pub(in crate::app::ai) subagent_task: Option<&'a crate::app::ai::task_bar::UiManagedTask>,
    pub(in crate::app::ai) p: &'a Palette,
}

/// 卡片展开/收起回调类型。
pub(in crate::app::ai) type ToggleCardCallback = Rc<dyn Fn(&mut Window, &mut App)>;
/// 回滚到指定轮次之前的回调类型。
pub(in crate::app::ai) type RollbackCallback = Rc<dyn Fn(String, &mut Window, &mut App)>;
/// 点击消息内图片缩略图触发大图预览的回调类型 `(title, base64)`。
pub(in crate::app::ai) type PreviewImageCallback =
    Rc<dyn Fn(String, String, &mut Window, &mut App)>;

/// 消息项交互回调集合。
#[derive(Clone)]
pub(in crate::app::ai) struct ChatItemCallbacks {
    pub(in crate::app::ai) on_toggle_expand: ToggleCardCallback,
    pub(in crate::app::ai) on_toggle_full_output: ToggleCardCallback,
    pub(in crate::app::ai) on_rollback: RollbackCallback,
    pub(in crate::app::ai) on_preview_image: PreviewImageCallback,
    pub(in crate::app::ai) code: CodeBlockCallbacks,
}

/// 渲染单条消息项。
pub(in crate::app::ai) fn render_chat_item(
    params: ChatItemRenderParams<'_>,
    callbacks: ChatItemCallbacks,
) -> AnyElement {
    let p = params.p;
    match params.item {
        UiChatItem::UserText {
            id,
            text,
            images,
            timestamp,
        } => render_user_bubble(id, text, images, *timestamp, &params, &callbacks),
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
            cached_segments,
        } => render_assistant_message(
            AssistantMessageParams {
                id,
                text,
                cached_segments: cached_segments
                    .as_ref()
                    .map(StreamingMarkdownCache::as_slice),
                model: model.as_deref(),
                tokens_per_sec: *tokens_per_sec,
                streaming: *streaming,
            },
            &params,
            &callbacks,
        ),
        UiChatItem::ToolCard { .. } => {
            crate::app::ai::tool_card::render_tool_card(&params, callbacks)
        }
        UiChatItem::AuditNotice {
            approved,
            label,
            host_label,
            ..
        } => render_audit_notice(*approved, label, host_label, p),
    }
}

fn render_audit_notice(approved: bool, label: &str, host_label: &str, p: &Palette) -> AnyElement {
    let (icon, color) = if approved {
        (UiIcon::CHECK, p.success)
    } else {
        (UiIcon::CLOSE, p.danger)
    };
    let mut row = h_flex()
        .w_full()
        .px(rems(space::XS))
        .py(px(3.0))
        .gap(px(6.0))
        .items_center()
        .justify_between()
        .rounded(p.radius.sm)
        .bg(p.secondary.opacity(0.6))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.0))
                .items_center()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(color)
                        .child(AppIconElement::new(AppIcon::Ui(icon)).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(label.to_owned()),
                ),
        );
    if !host_label.trim().is_empty() {
        row = row.child(
            div()
                .flex_shrink_0()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(p.radius.sm)
                .bg(p.background)
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child(host_label.to_owned()),
        );
    }
    row.into_any_element()
}

/// 将预解析的 UTC 时间戳格式化为简洁的中文相对时间；跨天时展示本地月日时分。
fn format_relative_timestamp(
    utc: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let secs = now.signed_duration_since(utc).num_seconds();
    if secs < 60 {
        rust_i18n::t!("ai.time_just_now").to_string()
    } else if secs < 3600 {
        rust_i18n::t!("ai.time_mins_ago", count = secs / 60).to_string()
    } else if secs < 86_400 {
        rust_i18n::t!("ai.time_hours_ago", count = secs / 3600).to_string()
    } else {
        utc.with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string()
    }
}

pub(super) fn copy_icon_and_tip(is_copied: bool) -> (UiIcon, String) {
    if is_copied {
        (UiIcon::CHECK, rust_i18n::t!("ai.copied").to_string())
    } else {
        (UiIcon::COPY, rust_i18n::t!("ai.copy").to_string())
    }
}

fn render_user_bubble(
    id: &str,
    text: &str,
    images: &[crate::app::ai::state::UiChatImage],
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    params: &ChatItemRenderParams<'_>,
    callbacks: &ChatItemCallbacks,
) -> AnyElement {
    let p = params.p;
    let copy_key = format!("user-{id}");
    let is_copied = params.copied_key == Some(copy_key.as_str());
    let (copy_icon, copy_tip) = copy_icon_and_tip(is_copied);
    let rb_tip = rust_i18n::t!("ai.rollback_turn").to_string();
    let on_copy = Rc::clone(&callbacks.code.on_copy);
    let copied_text = text.to_owned();
    let time_label = timestamp
        .map(|ts| format_relative_timestamp(ts, params.now))
        .unwrap_or_default();

    let mut action_row = h_flex()
        .pr(px(4.0))
        .justify_end()
        .items_center()
        .gap(px(4.0));
    if !time_label.is_empty() {
        action_row = action_row.child(
            div()
                .mr(px(4.0))
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child(time_label),
        );
    }
    action_row = action_row
        .child(
            Button::new(SharedString::from(format!("ai-copy-user-{id}")))
                .ghost()
                .xsmall()
                .accessibility_label(copy_tip.clone())
                .tooltip(copy_tip)
                .child(
                    div()
                        .text_color(if is_copied { p.success } else { p.text.muted })
                        .child(AppIconElement::new(AppIcon::Ui(copy_icon)).small()),
                )
                .on_click(move |_, w, cx| on_copy(copy_key.clone(), copied_text.clone(), w, cx)),
        )
        .child({
            let entry_id = id.to_owned();
            let on_rb = Rc::clone(&callbacks.on_rollback);
            Button::new(SharedString::from(format!("ai-rb-user-{id}")))
                .ghost()
                .xsmall()
                .accessibility_label(rb_tip.clone())
                .tooltip(rb_tip)
                .child(
                    div()
                        .text_color(p.text.muted)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::UNDO)).small()),
                )
                .on_click(move |_, w, cx| on_rb(entry_id.clone(), w, cx))
        });

    let mut col = v_flex().w_full().items_end().gap(px(4.0));
    if !images.is_empty() {
        let mut img_row = h_flex()
            .max_w(rems(32.0))
            .flex_wrap()
            .justify_end()
            .gap(px(6.0));
        for (idx, item_img) in images.iter().enumerate() {
            let preview = Rc::clone(&callbacks.on_preview_image);
            let title = format!("image-{}.png", idx + 1);
            let payload = item_img.base64.clone();
            let mut card = div()
                .id(SharedString::from(format!("ai-user-img-{id}-{idx}")))
                .w(px(120.0))
                .h(px(88.0))
                .rounded(p.radius.md)
                .border_1()
                .border_color(p.stroke.faint)
                .bg(p.surface.subtle)
                .overflow_hidden()
                .cursor_pointer()
                .hover(|s| s.border_color(p.primary.opacity(0.7)))
                .on_click(move |_, w, cx| preview(title.clone(), payload.clone(), w, cx));
            if let Some(thumb) = &item_img.thumb {
                card = card.child(
                    img(ImageSource::Render(thumb.clone()))
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                );
            } else {
                card = card
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(p.text.muted)
                    .child(
                        AppIconElement::new(AppIcon::Custom(transom_ui_icons::CustomIcon::IMAGE))
                            .small(),
                    );
            }
            img_row = img_row.child(card);
        }
        col = col.child(img_row);
    }
    col.child(
        div()
            .max_w(rems(32.0))
            .px(rems(space::MD))
            .py(rems(space::SM))
            .rounded(p.radius.lg)
            .bg(p.surface.subtle)
            .border_1()
            .border_color(p.stroke.faint)
            .text_size(rems(typography::BODY))
            .line_height(relative(1.55))
            .text_color(p.foreground)
            .child(text.to_owned()),
    )
    .child(action_row)
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
        .rounded(p.radius.md)
        .px(rems(space::SM))
        .py(px(5.0))
        .bg(if streaming {
            p.tone.primary_subtle
        } else {
            p.surface.ghost
        })
        .gap(px(4.0))
        .child(
            h_flex()
                .id(SharedString::from(format!("ai-reason-{id}")))
                .py(px(2.0))
                .gap(px(6.0))
                .items_center()
                .justify_between()
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .on_click(move |_, window, cx| on_toggle(window, cx))
                .child(
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(
                            div()
                                .text_color(if streaming { p.primary } else { p.text.muted })
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::LIGHTNING)).small()),
                        )
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if streaming {
                                    p.foreground
                                } else {
                                    p.text.subtle
                                })
                                .child(label),
                        ),
                )
                .child(
                    div()
                        .text_color(p.text.muted)
                        .child(AppIconElement::new(AppIcon::Ui(chevron)).small()),
                ),
        );
    if streaming || expanded {
        box_el = box_el.child(
            div()
                .id(SharedString::from(format!("ai-reason-body-{id}")))
                .ml(px(5.0))
                .pl(rems(space::SM))
                .py(px(2.0))
                .max_h(rems(16.0))
                .overflow_y_scroll()
                .border_l_2()
                .border_color(if streaming {
                    p.primary.opacity(0.45)
                } else {
                    p.stroke.muted
                })
                .text_size(rems(typography::CAPTION))
                .line_height(relative(1.55))
                .text_color(p.text.subtle)
                .child(text.to_owned()),
        );
    }
    box_el.into_any_element()
}
