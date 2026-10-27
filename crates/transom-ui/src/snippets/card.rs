//! 快捷命令弹出的卡片视图、命令单行组件与预览截断逻辑。

use super::SNIPPET_PREVIEW_MAX_CHARS;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::Snippet;

/// 摘要命令预览：取第一行并限制长度，避免多行脚本撑破卡片。
pub(crate) fn snippet_preview(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or_default().trim();
    let mut preview: String = first_line.chars().take(SNIPPET_PREVIEW_MAX_CHARS).collect();
    if command.contains('\n') || first_line.chars().count() > SNIPPET_PREVIEW_MAX_CHARS {
        preview.push_str(" …");
    }
    preview
}

/// 新增/编辑按钮统一行为：先收起卡片，再打开单条命令弹窗。
fn snippets_form_button(
    button: Button,
    app: &Entity<AppShell>,
    existing: Option<Snippet>,
) -> Button {
    let app = app.clone();
    button.on_click(move |_, window, cx| {
        let existing = existing.clone();
        app.update(cx, |this, cx| this.open_snippet_form(window, cx, existing));
    })
}

/// 卡片里的一行命令：名称与命令预览同行，点击按当前执行模式应用并收起卡片。
///
/// 行底色与悬浮底色参照会话列表：默认浅色块加明显一档的悬停色，让每一行都像
/// 可点击的列表项，而不是飘在弹层里的两行文字。编辑与删除按钮单独中止冒泡，
/// 避免点图标时顺手把命令填进终端。
fn snippets_row(app: &Entity<AppShell>, item: &Snippet, p: &Palette) -> AnyElement {
    let row_app = app.clone();
    let row_item = item.clone();
    let edit_app = app.clone();
    let edit_item = item.clone();
    let delete_app = app.clone();
    let item_id = item.id;
    h_flex()
        .id(ElementId::Name(format!("snippet-row-{}", item.id).into()))
        .w_full()
        .min_w_0()
        .h(px(30.0))
        .gap_2()
        .px_2()
        .items_center()
        .rounded(p.radius.md)
        .bg(p.muted)
        .cursor_pointer()
        .hover(|style| style.bg(p.foreground.opacity(0.10)))
        .on_click(move |_, _, cx| {
            let item = row_item.clone();
            row_app.update(cx, |this, cx| {
                let execute = this.settings.snippets.execute_directly;
                this.apply_snippet(&item, execute, cx);
                this.snippet_panel.open = false;
                cx.notify();
            });
        })
        .child(
            div()
                .w(px(80.0))
                .flex_none()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(item.name.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(snippet_preview(&item.command)),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .items_center()
                .child(
                    Button::new(ElementId::Name(format!("snippet-edit-{}", item.id).into()))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Pencil)
                        .tooltip(rust_i18n::t!("common.edit").to_string())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let item = edit_item.clone();
                            edit_app.update(cx, |this, cx| {
                                this.open_snippet_form(window, cx, Some(item));
                            });
                        }),
                )
                .child(
                    Button::new(ElementId::Name(
                        format!("snippet-delete-{}", item.id).into(),
                    ))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Trash)
                    .tooltip(rust_i18n::t!("common.delete").to_string())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        delete_app.update(cx, |this, cx| this.delete_snippet(item_id, cx));
                    }),
                ),
        )
        .into_any_element()
}

/// 卡片本体：标题、命令列表/空态、底部执行模式与新增入口。
pub(super) fn render_snippets_card(
    app: &Entity<AppShell>,
    items: &[Snippet],
    execute_directly: bool,
    p: &Palette,
) -> AnyElement {
    let list = if items.is_empty() {
        v_flex()
            .items_center()
            .gap_2()
            .py_4()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("snippets.empty").to_string()),
            )
            .child(snippets_form_button(
                Button::new("snippets-empty-add")
                    .link()
                    .xsmall()
                    .label(rust_i18n::t!("snippets.add").to_string()),
                app,
                None,
            ))
            .into_any_element()
    } else {
        // 行高 30、间距 4：六行 200，八行 268。少于此数保持六行高度，
        // 多于八行在卡片内滚动。
        v_flex()
            .id("snippets-card-list")
            .gap_1()
            .min_h(px(200.0))
            .max_h(px(268.0))
            .overflow_y_scrollbar()
            .children(items.iter().map(|item| snippets_row(app, item, p)))
            .into_any_element()
    };

    // 开启直接执行时用强调色提示当前点击会立即发送，避免用户忘记开关状态。
    let label_color = if execute_directly {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    };
    let hint = if execute_directly {
        rust_i18n::t!("snippets.execute_directly_on_hint").to_string()
    } else {
        rust_i18n::t!("snippets.execute_directly_off_hint").to_string()
    };
    let switch = Switch::new("snippets-execute-directly")
        .small()
        .checked(execute_directly)
        .on_click({
            let app = app.clone();
            move |checked, _, cx| {
                let checked = *checked;
                app.update(cx, |this, cx| {
                    this.settings.snippets.execute_directly = checked;
                    this.persist_layout_settings(cx);
                });
            }
        });

    v_flex()
        .w(px(280.0))
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(rust_i18n::t!("snippets.title").to_string()),
        )
        .child(list)
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_2()
                .border_t_1()
                .border_color(p.border)
                .pt_2()
                .child(
                    h_flex().gap_2().items_center().child(switch).child(
                        div()
                            .id("snippets-execute-hint")
                            .text_xs()
                            .text_color(label_color)
                            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                            .child(rust_i18n::t!("snippets.execute_directly").to_string()),
                    ),
                )
                .child(snippets_form_button(
                    Button::new("snippets-add")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Plus)
                        .label(rust_i18n::t!("snippets.add").to_string()),
                    app,
                    None,
                )),
        )
        .into_any_element()
}
