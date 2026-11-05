//! 原始 Inspect 配置 JSON 查看器：格式化高亮、关键词搜索与一键复制。

use super::*;
use gpui_kit::ClipboardItem;

/// 渲染原始 Inspect JSON 视图。
pub(crate) fn render_raw_inspect(
    tab_id: TabId,
    inspect_json: &str,
    search_input: &Entity<InputState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if inspect_json.trim().is_empty() {
        return centered_hint(rust_i18n::t!("container.detail.no_inspect").to_string(), p);
    }

    let search_keyword = search_input.read(cx).value().trim().to_lowercase();
    let lines: Vec<&str> = inspect_json.lines().collect();
    let total_lines = lines.len();

    // 统计匹配行数
    let matching_count = if search_keyword.is_empty() {
        total_lines
    } else {
        lines
            .iter()
            .filter(|line| line.to_lowercase().contains(&search_keyword))
            .count()
    };

    let copy_text = inspect_json.to_string();
    let copy_btn = Button::new(ElementId::Name(
        format!("btn-copy-raw-inspect-{tab_id}").into(),
    ))
    .ghost()
    .xsmall()
    .icon(IconName::Copy)
    .label(rust_i18n::t!("container.detail.copy_inspect").to_string())
    .on_click(cx.listener({
        let copy_text = copy_text.clone();
        move |this, _, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
            this.show_toast(
                rust_i18n::t!("container.detail.copied").to_string(),
                true,
                window,
                cx,
            );
        }
    }));

    let stats_label = if search_keyword.is_empty() {
        rust_i18n::t!(
            "container.inspect.stats_total",
            lines = total_lines,
            bytes = inspect_json.len()
        )
        .to_string()
    } else {
        rust_i18n::t!(
            "container.inspect.stats_matched",
            matched = matching_count,
            total = total_lines
        )
        .to_string()
    };

    let back_btn = Button::new(ElementId::Name(
        format!("btn-back-overview-{tab_id}").into(),
    ))
    .ghost()
    .xsmall()
    .icon(IconName::LayoutDashboard)
    .label(rust_i18n::t!("container.detail.overview").to_string())
    .on_click(cx.listener(move |this, _, _, cx| {
        if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
            workspace.overview_raw = false;
        }
        cx.notify();
    }));

    let toolbar = h_flex()
        .justify_between()
        .items_center()
        .gap_2()
        .pb_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .flex_1()
                .child(
                    div()
                        .w_64()
                        .child(Input::new(search_input).cleanable(true).small()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(stats_label),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(copy_btn)
                .child(back_btn),
        );

    // 逐行渲染带有行号与关键词高亮的代码块
    let mut code_body = v_flex()
        .gap_0()
        .font_family(p.mono_font_family.clone())
        .text_size(p.mono_font_size);

    for (idx, line) in lines.iter().enumerate() {
        let is_matched =
            !search_keyword.is_empty() && line.to_lowercase().contains(&search_keyword);

        let mut row = h_flex()
            .items_start()
            .gap_2()
            .py_0p5()
            .px_1()
            .rounded(p.radius.sm);

        if is_matched {
            row = row.bg(p.primary.opacity(0.18));
        }

        // 行号
        let line_num = div()
            .w_10()
            .text_right()
            .flex_none()
            .text_xs()
            .text_color(p.muted_foreground.opacity(0.6))
            .child((idx + 1).to_string());

        // 行内容
        let line_content = div()
            .flex_1()
            .min_w_0()
            .text_xs()
            .text_color(if is_matched {
                p.foreground
            } else {
                p.foreground.opacity(0.9)
            })
            .child(line.to_string());

        code_body = code_body.child(row.child(line_num).child(line_content));
    }

    let code_card = div()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.2))
        .border_1()
        .border_color(p.border.opacity(0.5))
        .child(code_body);

    scroll_body(v_flex().child(toolbar).child(code_card), p).into_any_element()
}
