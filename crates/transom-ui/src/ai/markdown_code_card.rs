//! AI 消息流 Markdown 围栏代码块卡片渲染：支持语言徽章、行数统计、流式标识、
//! 一键复制、填入终端与保存为快捷指令。

use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui_kit::base::h_flex;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use parking_lot::Mutex;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, DomainIcon, UiIcon};

use super::markdown_blocks::{MarkdownRenderParams, is_shell_language};
use crate::theme::tokens::{space, typography};

const MAX_CACHED_LINE_NUMBERS: usize = 512;

static LINE_NUMBERS_CACHE: LazyLock<Mutex<HashMap<usize, SharedString>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn format_line_numbers(line_count: usize) -> SharedString {
    let mut buf = String::with_capacity(line_count.saturating_mul(4));
    for n in 1..=line_count {
        if n > 1 {
            buf.push('\n');
        }
        let _ = write!(&mut buf, "{n}");
    }
    SharedString::from(buf)
}

fn cached_line_numbers(line_count: usize) -> SharedString {
    if line_count > MAX_CACHED_LINE_NUMBERS {
        return format_line_numbers(line_count);
    }
    if let Some(hit) = LINE_NUMBERS_CACHE.lock().get(&line_count) {
        return hit.clone();
    }
    let formatted = format_line_numbers(line_count);
    LINE_NUMBERS_CACHE
        .lock()
        .insert(line_count, formatted.clone());
    formatted
}

/// 渲染 Markdown 围栏代码块为轻量化交互卡片。
pub(super) fn render_code_card(
    params: &MarkdownRenderParams<'_>,
    idx: usize,
    lang: Option<&str>,
    code: &str,
    unclosed: bool,
) -> AnyElement {
    let p = params.p;
    let block_key = format!("code-{}-{idx}", params.msg_id);
    let is_copied = params.copied_key == Some(block_key.as_str());
    let is_shell = is_shell_language(lang);
    let lang_label = lang
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| rust_i18n::t!("ai.code_block_default").to_string());

    let icon = if is_shell {
        AppIcon::Domain(DomainIcon::TERMINAL)
    } else {
        AppIcon::Custom(CustomIcon::CPU)
    };

    let line_count = code.lines().count().max(1);
    let mut left = h_flex().gap(px(6.0)).items_center().child(
        h_flex()
            .gap(px(5.0))
            .items_center()
            .child(
                div()
                    .text_color(p.text.muted)
                    .child(AppIconElement::new(icon).small()),
            )
            .child(
                div()
                    .font_family(p.mono_font_family.clone())
                    .text_size(rems(typography::CAPTION))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text.subtle)
                    .child(lang_label),
            )
            .child(
                div()
                    .font_family(p.mono_font_family.clone())
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.disabled)
                    .child(format!("{line_count}L")),
            ),
    );
    if unclosed {
        left = left.child(
            h_flex()
                .px(px(6.0))
                .py(px(1.0))
                .gap(px(4.0))
                .items_center()
                .rounded_full()
                .bg(p.tone.primary_subtle)
                .child(div().w(px(5.0)).h(px(5.0)).rounded_full().bg(p.primary))
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.primary)
                        .child(rust_i18n::t!("ai.streaming_code").to_string()),
                ),
        );
    }

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
    let on_copy = Rc::clone(&params.callbacks.on_copy);
    let copy_code = code.to_owned();
    let copy_key = block_key.clone();

    let mut actions = h_flex().gap(px(4.0)).items_center();
    if is_shell && !unclosed {
        if params.has_active_terminal {
            let on_send = Rc::clone(&params.callbacks.on_send_to_terminal);
            let send_code = code.to_owned();
            actions = actions.child(
                Button::new(SharedString::from(format!("ai-send-term-{block_key}")))
                    .ghost()
                    .xsmall()
                    .label(rust_i18n::t!("ai.send_to_terminal").to_string())
                    .on_click(move |_, w, cx| on_send(send_code.clone(), w, cx)),
            );
        }
        let on_snip = Rc::clone(&params.callbacks.on_save_snippet);
        let snip_code = code.to_owned();
        let snip_name = code
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(32)
            .collect::<String>();
        actions = actions.child(
            Button::new(SharedString::from(format!("ai-snip-code-{block_key}")))
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.save_as_snippet").to_string())
                .on_click(move |_, w, cx| on_snip(snip_name.clone(), snip_code.clone(), w, cx)),
        );
    }
    actions = actions.child(
        Button::new(SharedString::from(format!("ai-copy-code-{block_key}")))
            .ghost()
            .xsmall()
            .accessibility_label(copy_tip.clone())
            .tooltip(copy_tip)
            .child(
                div()
                    .text_color(if is_copied { p.success } else { p.text.muted })
                    .child(AppIconElement::new(AppIcon::Ui(copy_icon)).small()),
            )
            .on_click(move |_, w, cx| on_copy(copy_key.clone(), copy_code.clone(), w, cx)),
    );

    let header = h_flex()
        .w_full()
        .px(rems(space::SM))
        .py(px(4.0))
        .justify_between()
        .items_center()
        .bg(p.surface.faint)
        .border_b_1()
        .border_color(p.stroke.faint)
        .child(left)
        .child(actions);

    let line_numbers = cached_line_numbers(line_count);
    let body_text = if code.is_empty() {
        " ".to_owned()
    } else {
        code.to_owned()
    };

    v_flex()
        .w_full()
        .rounded(p.radius.md)
        .overflow_hidden()
        .border_1()
        .border_color(if unclosed {
            p.primary.opacity(0.35)
        } else {
            p.stroke.faint
        })
        .bg(p.surface.ghost)
        .child(header)
        .child(
            h_flex()
                .id(SharedString::from(format!("ai-code-scroll-{block_key}")))
                .w_full()
                .max_h(rems(26.0))
                .overflow_y_scroll()
                .items_start()
                .child(
                    div()
                        .flex_shrink_0()
                        .px(rems(space::XS))
                        .py(rems(space::XS))
                        .border_r_1()
                        .border_color(p.stroke.faint)
                        .bg(p.surface.ghost)
                        .font_family(p.mono_font_family.clone())
                        .text_size(rems(typography::CAPTION))
                        .line_height(relative(1.5))
                        .text_color(p.text.disabled)
                        .text_right()
                        .child(line_numbers),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("ai-code-body-{block_key}")))
                        .flex_1()
                        .min_w_0()
                        .px(rems(space::SM))
                        .py(rems(space::XS))
                        .overflow_x_scroll()
                        .font_family(p.mono_font_family.clone())
                        .text_size(rems(typography::CAPTION))
                        .line_height(relative(1.5))
                        .text_color(p.foreground)
                        .child(body_text),
                ),
        )
        .into_any_element()
}
