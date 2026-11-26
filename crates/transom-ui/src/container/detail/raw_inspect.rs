//! 原始 Inspect 配置 JSON 查看器：格式化高亮、关键词搜索与一键复制。
//!
//! 设计原理（Why）：渲染函数只持有工作区实体句柄与只读快照，复制提示写回实体的
//! `notice` 字段、返回结构化视图改写 `overview_raw`，由外壳渲染帧统一消费；
//! 这样原始视图不必再依赖 `Context<AppShell>`，也不会在渲染期反查外壳。

use super::*;
use crate::container::ContainerWorkspace;
use crate::search::contains_case_insensitive;
use crate::widgets::action_button;
use gpui_kit::ClipboardItem;
use transom_ui_icons::UiIcon;

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
    let keyword = search.keyword.trim().to_lowercase();
    let lines: Vec<&str> = inspect_json.lines().collect();
    let total_lines = lines.len();
    let matched_lines = if keyword.is_empty() {
        total_lines
    } else {
        lines
            .iter()
            .filter(|line| contains_case_insensitive(line, &keyword))
            .count()
    };
    let toolbar = render_inspect_toolbar(
        view,
        inspect_json,
        search.input,
        &keyword,
        matched_lines,
        total_lines,
        p,
    );
    let code_card = render_inspect_code(&lines, &keyword, p);
    scroll_body(v_flex().child(toolbar).child(code_card), p).into_any_element()
}

fn render_inspect_toolbar(
    view: &Entity<ContainerWorkspace>,
    inspect_json: &str,
    input: &Entity<InputState>,
    keyword: &str,
    matched_lines: usize,
    total_lines: usize,
    p: &Palette,
) -> AnyElement {
    let stats = inspect_stats_label(keyword, matched_lines, total_lines, inspect_json.len());
    h_flex()
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
                        .child(Input::new(input).cleanable(true).small()),
                )
                .child(div().text_xs().text_color(p.muted_foreground).child(stats)),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(render_copy_inspect_button(view, inspect_json))
                .child(render_back_overview_button(view)),
        )
        .into_any_element()
}

fn inspect_stats_label(
    keyword: &str,
    matched_lines: usize,
    total_lines: usize,
    bytes: usize,
) -> String {
    if keyword.is_empty() {
        return rust_i18n::t!(
            "container.inspect.stats_total",
            lines = total_lines,
            bytes = bytes
        )
        .to_string();
    }
    rust_i18n::t!(
        "container.inspect.stats_matched",
        matched = matched_lines,
        total = total_lines
    )
    .to_string()
}

fn render_copy_inspect_button(view: &Entity<ContainerWorkspace>, inspect_json: &str) -> AnyElement {
    let view = view.clone();
    let copy_text = inspect_json.to_string();
    action_button(
        ElementId::Name("btn-copy-raw-inspect".into()),
        UiIcon::COPY,
        rust_i18n::t!("container.detail.copy_inspect").to_string(),
    )
    .ghost()
    .on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
        let message = rust_i18n::t!("container.detail.copied").to_string();
        view.update(cx, |workspace, cx| {
            workspace.state.notice = Some(message);
            cx.notify();
        });
    })
    .into_any_element()
}

fn render_back_overview_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let view = view.clone();
    action_button(
        ElementId::Name("btn-back-overview".into()),
        UiIcon::EDITOR_PREVIEW,
        rust_i18n::t!("container.detail.overview").to_string(),
    )
    .ghost()
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.state.overview_raw = false;
            cx.notify();
        });
    })
    .into_any_element()
}

fn render_inspect_code(lines: &[&str], keyword: &str, p: &Palette) -> AnyElement {
    let mut body = v_flex()
        .gap_0()
        .font_family(p.mono_font_family.clone())
        .text_size(p.mono_font_size);
    for (index, line) in lines.iter().enumerate() {
        body = body.child(render_inspect_line(index, line, keyword, p));
    }
    div()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.2))
        .border_1()
        .border_color(p.stroke.strong)
        .child(body)
        .into_any_element()
}

fn render_inspect_line(index: usize, line: &str, keyword: &str, p: &Palette) -> AnyElement {
    let matched = contains_case_insensitive(line, keyword);
    let mut row = h_flex()
        .items_start()
        .gap_2()
        .py_0p5()
        .px_1()
        .rounded(p.radius.sm);
    if matched {
        row = row.bg(p.primary.opacity(0.18));
    }
    let line_number = div()
        .w_10()
        .text_right()
        .flex_none()
        .text_xs()
        .text_color(p.muted_foreground.opacity(0.6))
        .child((index + 1).to_string());
    let content = div()
        .flex_1()
        .min_w_0()
        .text_xs()
        .text_color(if matched {
            p.foreground
        } else {
            p.foreground.opacity(0.9)
        })
        .child(line.to_string());
    row.child(line_number).child(content).into_any_element()
}
