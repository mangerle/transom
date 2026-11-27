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
    pub(super) full_output_expanded: bool,
    pub(super) copied_key: Option<&'a str>,
    pub(super) has_active_terminal: bool,
    /// 若当前项是 `delegate_task`，则关联对应的实时子代理托管任务状态。
    pub(super) subagent_task: Option<&'a super::task_bar::UiManagedTask>,
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
    pub(super) on_toggle_full_output: ToggleCardCallback,
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
        UiChatItem::UserText {
            id,
            text,
            timestamp,
        } => render_user_bubble(id, text, timestamp.as_deref(), &params, &callbacks),
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
            id,
            text,
            cached_segments.as_deref().map(Vec::as_slice),
            model.as_deref(),
            *tokens_per_sec,
            *streaming,
            &params,
            &callbacks,
        ),
        UiChatItem::ToolCard { .. } => super::tool_card::render_tool_card(&params, callbacks),
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

/// 将 RFC3339 时间戳格式化为简洁的中文相对时间；跨天时展示本地月日时分，解析失败返回空串。
fn format_relative_timestamp(raw: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return String::new();
    };
    let utc = dt.with_timezone(&chrono::Utc);
    let secs = now.signed_duration_since(utc).num_seconds();
    if secs < 60 {
        "刚刚".to_owned()
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86_400 {
        format!("{} 小时前", secs / 3600)
    } else {
        dt.with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string()
    }
}

fn render_user_bubble(
    id: &str,
    text: &str,
    timestamp: Option<&str>,
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
    let copy_tip = if is_copied {
        rust_i18n::t!("ai.copied").to_string()
    } else {
        rust_i18n::t!("ai.copy").to_string()
    };
    let rb_tip = rust_i18n::t!("ai.rollback_turn").to_string();
    let on_copy = Rc::clone(&callbacks.code.on_copy);
    let copied_text = text.to_owned();
    let time_label = timestamp
        .map(|ts| format_relative_timestamp(ts, chrono::Utc::now()))
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

    v_flex()
        .w_full()
        .items_end()
        .gap(px(4.0))
        .child(
            div()
                .max_w(rems(28.0))
                .px(px(14.0))
                .py(px(9.0))
                .rounded(px(16.0))
                .bg(p.secondary)
                .border_1()
                .border_color(p.divider(p.border).opacity(0.6))
                .text_size(rems(typography::BODY))
                .line_height(relative(1.45))
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
    let mut box_el = v_flex().w_full().gap(px(4.0)).child(
        h_flex()
            .id(SharedString::from(format!("ai-reason-{id}")))
            .py(px(2.0))
            .gap(px(5.0))
            .items_center()
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
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
                .ml(px(6.0))
                .pl(rems(space::SM))
                .py(px(2.0))
                .border_l_2()
                .border_color(p.divider(p.border))
                .text_size(rems(typography::CAPTION))
                .line_height(relative(1.45))
                .text_color(p.muted_foreground)
                .child(text.to_owned()),
        );
    }
    box_el.into_any_element()
}

fn render_assistant_message(
    id: &str,
    text: &str,
    cached_segments: Option<&[super::markdown_blocks::MarkdownSegment]>,
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
            cached_segments,
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
        let copy_tip = if is_copied {
            rust_i18n::t!("ai.copied").to_string()
        } else {
            rust_i18n::t!("ai.copy").to_string()
        };
        let on_copy = Rc::clone(&callbacks.code.on_copy);
        let full_text = text.to_owned();

        col = col.child(
            h_flex()
                .w_full()
                .pt(px(4.0))
                .justify_between()
                .items_center()
                .child(
                    div()
                        .opacity(0.85)
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(footer_parts.join(" · ")),
                )
                .child(
                    Button::new(SharedString::from(format!("ai-copy-asst-{id}")))
                        .ghost()
                        .xsmall()
                        .accessibility_label(copy_tip.clone())
                        .tooltip(copy_tip)
                        .child(
                            div()
                                .text_color(if is_copied { p.success } else { p.text.muted })
                                .child(AppIconElement::new(AppIcon::Ui(copy_icon)).small()),
                        )
                        .on_click(move |_, w, cx| {
                            on_copy(copy_key.clone(), full_text.clone(), w, cx);
                        }),
                ),
        );
    }
    col.into_any_element()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};

    use super::format_relative_timestamp;

    #[test]
    fn formats_relative_timestamps_across_ranges() {
        let now = Utc.with_ymd_and_hms(2026, 11, 27, 10, 0, 0).unwrap();

        assert_eq!(
            format_relative_timestamp("2026-11-27T09:59:30Z", now),
            "刚刚"
        );
        assert_eq!(
            format_relative_timestamp("2026-11-27T09:45:00Z", now),
            "15 分钟前"
        );
        assert_eq!(
            format_relative_timestamp("2026-11-27T07:00:00Z", now),
            "3 小时前"
        );
        let older = format_relative_timestamp("2026-11-25T08:30:00Z", now);
        assert!(!older.is_empty() && older.contains(':'));
        assert_eq!(format_relative_timestamp("not-a-timestamp", now), "");
    }
}
