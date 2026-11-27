//! 对话流内的极简工具与 Shell 命令输出卡片渲染。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon, UiIcon};

use super::cards::{ChatItemCallbacks, ChatItemRenderParams};
use super::state::UiChatItem;
use crate::theme::tokens::{space, typography};

fn tool_row_icon(tool_name: &str) -> AppIcon {
    match tool_name {
        "remote_exec" | "remote_exec_bulk" | "container_ops" => {
            AppIcon::Domain(DomainIcon::TERMINAL)
        }
        "remote_fs" | "transfer_file" | "read_output" => AppIcon::Ui(UiIcon::LIST_FILES),
        _ => AppIcon::Ui(UiIcon::RUN),
    }
}

/// 渲染对话流中的工具调用单行与可点击展开的 Shell / 详情框。
pub(super) fn render_tool_card(
    params: &ChatItemRenderParams<'_>,
    callbacks: ChatItemCallbacks,
) -> AnyElement {
    let p = params.p;
    let UiChatItem::ToolCard {
        id,
        tool_name,
        host_label,
        risk_label,
        summary,
        output,
        ok,
        duration_ms,
        diff_preview,
    } = params.item
    else {
        return div().into_any_element();
    };

    let is_cmd = matches!(
        tool_name.as_str(),
        "remote_exec" | "remote_exec_bulk" | "container_ops"
    );
    let on_toggle = callbacks.on_toggle_expand;
    let row_title = if summary.trim().is_empty() {
        format!("{tool_name} ({host_label})")
    } else {
        summary.clone()
    };

    let mut header = h_flex()
        .id(SharedString::from(format!("ai-tool-{id}")))
        .w_full()
        .px(rems(space::XS))
        .py(px(3.0))
        .gap(px(6.0))
        .items_center()
        .justify_between()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, window, cx| on_toggle(window, cx))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.0))
                .items_center()
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(if *ok { p.text.muted } else { p.danger })
                        .child(AppIconElement::new(tool_row_icon(tool_name)).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rems(typography::CAPTION))
                        .text_color(if params.expanded {
                            p.foreground
                        } else {
                            p.text.muted
                        })
                        .child(row_title),
                ),
        );

    let mut right = h_flex().flex_shrink_0().gap(px(4.0)).items_center();
    if !*ok {
        right = right.child(
            div()
                .text_color(p.danger)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::ERROR)).small()),
        );
    }
    right = right.child(
        div().text_color(p.text.muted).child(
            AppIconElement::new(AppIcon::Ui(if params.expanded {
                UiIcon::CHEVRON_DOWN
            } else {
                UiIcon::CHEVRON_RIGHT
            }))
            .small(),
        ),
    );
    header = header.child(right);

    let mut root = v_flex().w_full().gap(rems(space::XXS)).child(header);
    if tool_name == "remote_exec_bulk" {
        let rows = super::bulk_matrix::parse_bulk_matrix(output);
        if !rows.is_empty() {
            root = root.child(super::bulk_matrix::render_bulk_matrix(
                id,
                &rows,
                params.expanded,
                p,
            ));
            return root.into_any_element();
        }
    }
    if !params.expanded {
        return root.into_any_element();
    }

    let box_title = if is_cmd {
        rust_i18n::t!("ai.tool_shell_header", host = host_label).to_string()
    } else {
        format!("{tool_name} · {host_label}")
    };
    let mut actions = h_flex().gap(px(4.0)).items_center();
    if !output.trim().is_empty() {
        let out_key = format!("tool-out-{id}");
        let is_copied = params.copied_key == Some(out_key.as_str());
        let on_copy = Rc::clone(&callbacks.code.on_copy);
        let out_text = output.clone();
        let copy_tip = if is_copied {
            rust_i18n::t!("ai.copied").to_string()
        } else {
            rust_i18n::t!("ai.copy_output").to_string()
        };
        actions = actions.child(
            Button::new(SharedString::from(format!("ai-copy-out-{id}")))
                .ghost()
                .xsmall()
                .accessibility_label(copy_tip.clone())
                .tooltip(copy_tip)
                .child(
                    AppIconElement::new(AppIcon::Ui(if is_copied {
                        UiIcon::CHECK
                    } else {
                        UiIcon::COPY
                    }))
                    .small(),
                )
                .on_click(move |_, w, cx| on_copy(out_key.clone(), out_text.clone(), w, cx)),
        );
    }
    if matches!(tool_name.as_str(), "remote_exec" | "remote_exec_bulk")
        && !summary.trim().is_empty()
    {
        let cmd_name = summary.chars().take(32).collect::<String>();
        let cmd_text = summary.clone();
        let on_snip = Rc::clone(&callbacks.code.on_save_snippet);
        actions = actions.child(
            Button::new(SharedString::from(format!("ai-save-snip-{id}")))
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.save_as_snippet").to_string())
                .on_click(move |_, w, cx| on_snip(cmd_name.clone(), cmd_text.clone(), w, cx)),
        );
    }

    let mut body = v_flex()
        .id(SharedString::from(format!("ai-tool-out-{id}")))
        .max_h(rems(18.0))
        .overflow_y_scroll()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(px(4.0))
        .font_family("monospace")
        .text_size(rems(typography::CAPTION));
    if is_cmd && !summary.trim().is_empty() {
        body = body.child(div().text_color(p.text.muted).child(format!("$ {summary}")));
    }
    if let Some(diff) = diff_preview {
        body = body.child(div().text_color(p.muted_foreground).child(diff.clone()));
    }
    if !output.trim().is_empty() {
        body = body.child(div().text_color(p.foreground).child(output.clone()));
    }

    let status_icon = if *ok { UiIcon::CHECK } else { UiIcon::CLOSE };
    let status_color = if *ok { p.success } else { p.danger };
    let mut parts = vec![if *ok {
        rust_i18n::t!("ai.status_ok").to_string()
    } else {
        rust_i18n::t!("ai.status_fail").to_string()
    }];
    if !risk_label.trim().is_empty() {
        parts.push(risk_label.clone());
    }
    if *duration_ms > 0 {
        parts.push(format!("{duration_ms}ms"));
    }
    let status_label = parts.join(" · ");

    let shell_box = v_flex()
        .w_full()
        .rounded(p.radius.md)
        .bg(p.secondary)
        .border_1()
        .border_color(p.divider(p.border))
        .overflow_hidden()
        .child(
            h_flex()
                .w_full()
                .px(rems(space::SM))
                .py(px(4.0))
                .justify_between()
                .items_center()
                .border_b_1()
                .border_color(p.divider(p.border))
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(box_title),
                )
                .child(actions),
        )
        .child(body)
        .child(
            h_flex()
                .w_full()
                .px(rems(space::SM))
                .py(px(4.0))
                .justify_end()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .text_color(status_color)
                        .child(AppIconElement::new(AppIcon::Ui(status_icon)).small()),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(status_color)
                        .child(status_label),
                ),
        );
    root.child(shell_box).into_any_element()
}
