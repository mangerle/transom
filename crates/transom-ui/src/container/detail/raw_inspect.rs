//! 原始 Inspect 配置 JSON 查看器：格式化高亮、关键词搜索与一键复制。
//!
//! 设计原理（Why）：渲染函数只持有工作区实体句柄与只读快照，复制提示写回实体的
//! `notice` 字段、返回结构化视图改写 `overview_raw`，由外壳渲染帧统一消费；
//! 这样原始视图不必再依赖 `Context<AppShell>`，也不会在渲染期反查外壳。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::ClipboardItem;

/// 原始 inspect 工具栏的搜索输入框与当前关键词快照。
///
/// 设计原理（Why）：`InputState` 的当前值必须经 `App` 读取，纯渲染函数拿不到
/// `App`，因此由外壳在调用前把关键词一并快照进来；输入框实体仍用于渲染工具栏。
/// 代价是调用点必须先读一次输入框，换来渲染函数与外壳上下文彻底解耦。
#[derive(Clone, Copy)]
pub(crate) struct InspectSearch<'a> {
    pub(crate) input: &'a Entity<InputState>,
    pub(crate) keyword: &'a str,
}

/// 渲染原始 Inspect JSON 视图。
///
/// 设计原理（Why）：复制成功只把文案写进工作区 `notice`，返回结构化视图只改
/// `overview_raw`；两者都由外壳在渲染帧统一处理，不需要在渲染函数里持有窗口。
pub(crate) fn render_raw_inspect(
    view: &Entity<ContainerWorkspace>,
    inspect_json: &str,
    search: InspectSearch<'_>,
    p: &Palette,
) -> AnyElement {
    if inspect_json.trim().is_empty() {
        return centered_hint(rust_i18n::t!("container.detail.no_inspect").to_string(), p);
    }

    let search_keyword = search.keyword.trim().to_lowercase();
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

    let copy_btn = Button::new(ElementId::Name("btn-copy-raw-inspect".into()))
        .ghost()
        .xsmall()
        .icon(IconName::Copy)
        .label(rust_i18n::t!("container.detail.copy_inspect").to_string())
        .on_click({
            let view = view.clone();
            let copy_text = inspect_json.to_string();
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                let message = rust_i18n::t!("container.detail.copied").to_string();
                view.update(cx, |workspace, cx| {
                    workspace.state.notice = Some(message);
                    cx.notify();
                });
            }
        });

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

    let back_btn = Button::new(ElementId::Name("btn-back-overview".into()))
        .ghost()
        .xsmall()
        .icon(IconName::LayoutDashboard)
        .label(rust_i18n::t!("container.detail.overview").to_string())
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.state.overview_raw = false;
                    cx.notify();
                });
            }
        });

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
                        .child(Input::new(search.input).cleanable(true).small()),
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
