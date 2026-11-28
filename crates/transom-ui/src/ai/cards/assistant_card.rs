//! AI 助手回复正文卡片与时间戳格式测试。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement};

use super::{ChatItemCallbacks, ChatItemRenderParams, copy_icon_and_tip};
use crate::app::ai::markdown_blocks::{
    MarkdownRenderParams, MarkdownSegment, render_markdown_content,
};
use crate::theme::tokens::{space, typography};

/// 助手文本消息渲染参数。
pub(super) struct AssistantMessageParams<'a> {
    pub(super) id: &'a str,
    pub(super) text: &'a str,
    pub(super) cached_segments: Option<&'a [MarkdownSegment]>,
    pub(super) model: Option<&'a str>,
    pub(super) tokens_per_sec: Option<f32>,
    pub(super) streaming: bool,
}

pub(super) fn render_assistant_message(
    msg: AssistantMessageParams<'_>,
    params: &ChatItemRenderParams<'_>,
    callbacks: &ChatItemCallbacks,
) -> AnyElement {
    let AssistantMessageParams {
        id,
        text,
        cached_segments,
        model,
        tokens_per_sec,
        streaming,
    } = msg;
    let p = params.p;
    let content = render_markdown_content(MarkdownRenderParams {
        msg_id: id,
        text,
        cached_segments,
        streaming,
        copied_key: params.copied_key,
        has_active_terminal: params.has_active_terminal,
        p,
        callbacks: &callbacks.code,
    });
    let mut col = v_flex()
        .w_full()
        .py(rems(space::XS))
        .gap(rems(space::XS))
        .child(content);

    if streaming {
        col = col.child(
            h_flex()
                .pt(px(4.0))
                .gap(px(6.0))
                .items_center()
                .child(
                    div()
                        .w(px(12.0))
                        .h(px(12.0))
                        .rounded_full()
                        .bg(p.tone.primary_subtle)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().w(px(5.0)).h(px(5.0)).rounded_full().bg(p.primary)),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(rust_i18n::t!("ai.streaming_code").to_string()),
                ),
        );
    } else {
        let mut meta_bar = h_flex().gap(px(8.0)).items_center();
        if let Some(m) = model {
            meta_bar = meta_bar.child(
                div()
                    .px(px(6.0))
                    .py(px(1.5))
                    .rounded(p.radius.sm)
                    .bg(p.surface.faint)
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(m.to_owned()),
            );
        }
        if let Some(tps) = tokens_per_sec {
            meta_bar = meta_bar.child(
                div()
                    .font_family(p.mono_font_family.clone())
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.disabled)
                    .child(format!("{tps:.1} tok/s")),
            );
        }
        let copy_key = format!("asst-{id}");
        let is_copied = params.copied_key == Some(copy_key.as_str());
        let (copy_icon, copy_tip) = copy_icon_and_tip(is_copied);
        let on_copy = Rc::clone(&callbacks.code.on_copy);
        let full_text = text.to_owned();

        col = col.child(
            h_flex()
                .w_full()
                .pt(px(2.0))
                .justify_between()
                .items_center()
                .child(meta_bar)
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

    use super::super::format_relative_timestamp;

    #[test]
    fn formats_relative_timestamps_across_ranges() {
        let now = Utc.with_ymd_and_hms(2026, 11, 27, 10, 0, 0).unwrap();

        assert_eq!(
            format_relative_timestamp(Utc.with_ymd_and_hms(2026, 11, 27, 9, 59, 30).unwrap(), now),
            rust_i18n::t!("ai.time_just_now").as_ref()
        );
        assert_eq!(
            format_relative_timestamp(Utc.with_ymd_and_hms(2026, 11, 27, 9, 45, 0).unwrap(), now),
            rust_i18n::t!("ai.time_mins_ago", count = 15).as_ref()
        );
        assert_eq!(
            format_relative_timestamp(Utc.with_ymd_and_hms(2026, 11, 27, 7, 0, 0).unwrap(), now),
            rust_i18n::t!("ai.time_hours_ago", count = 3).as_ref()
        );
        let older =
            format_relative_timestamp(Utc.with_ymd_and_hms(2026, 11, 25, 8, 30, 0).unwrap(), now);
        assert!(!older.is_empty() && older.contains(':'));
    }
}
