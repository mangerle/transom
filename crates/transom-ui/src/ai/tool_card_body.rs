//! 工具卡片图标映射、子代理任务块与主机对比结果渲染。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon, UiIcon};

use super::cards::ChatItemRenderParams;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

pub(super) type ToggleOutputFn = Rc<dyn Fn(&mut Window, &mut App)>;

pub(super) fn tool_row_icon(tool_name: &str) -> AppIcon {
    match tool_name {
        "remote_exec" | "remote_exec_bulk" | "container_ops" => {
            AppIcon::Domain(DomainIcon::TERMINAL)
        }
        "remote_fs" | "transfer_file" | "read_output" => AppIcon::Ui(UiIcon::LIST_FILES),
        "session_catalog" => AppIcon::Ui(UiIcon::WEB_SERVER),
        "workspace_control" => AppIcon::Ui(UiIcon::LAYOUT),
        "delegate_task" => AppIcon::Ui(UiIcon::DATAFLOW),
        "compare_hosts" => AppIcon::Ui(UiIcon::DIFF),
        "search_knowledge" => AppIcon::Ui(UiIcon::SEARCH),
        _ => AppIcon::Ui(UiIcon::DEPENDENCIES),
    }
}

pub(super) fn render_subagent_task_block(
    params: &ChatItemRenderParams<'_>,
    on_toggle_full: ToggleOutputFn,
    p: &Palette,
) -> Option<AnyElement> {
    let sub = params.subagent_task?;
    let (st_label, st_color) = match sub.dto.status {
        transom_core::AiManagedTaskStatus::Running => (
            rust_i18n::t!("ai.subagent_status_running").to_string(),
            p.primary,
        ),
        transom_core::AiManagedTaskStatus::Succeeded => (
            rust_i18n::t!("ai.subagent_status_succeeded").to_string(),
            p.success,
        ),
        transom_core::AiManagedTaskStatus::Failed => (
            rust_i18n::t!("ai.subagent_status_failed").to_string(),
            p.danger,
        ),
        transom_core::AiManagedTaskStatus::Cancelled => (
            rust_i18n::t!("ai.subagent_status_cancelled").to_string(),
            p.text.muted,
        ),
    };
    let mut col = v_flex().w_full().gap(px(4.0)).child(
        h_flex()
            .w_full()
            .px(rems(space::XS))
            .py(px(4.0))
            .justify_between()
            .items_center()
            .rounded(p.radius.sm)
            .bg(p.surface.floating)
            .border_1()
            .border_color(p.divider(p.border))
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(st_color)
                    .font_weight(FontWeight::MEDIUM)
                    .child(
                        rust_i18n::t!(
                            "ai.subagent_target_line",
                            status = st_label.as_str(),
                            target = sub.dto.target_label.as_str()
                        )
                        .to_string(),
                    ),
            )
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(sub.dto.task_id.clone()),
            ),
    );
    if !sub.log.trim().is_empty() {
        col = col.child(super::diff_view::render_log_output(
            &sub.dto.task_id,
            &sub.log,
            params.full_output_expanded,
            p,
            move |w, cx| on_toggle_full(w, cx),
        ));
    }
    Some(col.into_any_element())
}

pub(super) fn render_compare_equal_banner(p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .px(rems(space::SM))
        .py(px(6.0))
        .gap(px(6.0))
        .items_center()
        .rounded(p.radius.sm)
        .bg(p.success.opacity(0.10))
        .border_1()
        .border_color(p.success.opacity(0.28))
        .child(
            div()
                .text_color(p.success)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small()),
        )
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.success)
                .child(rust_i18n::t!("ai.tool_compare_equal").to_string()),
        )
        .into_any_element()
}

pub(super) type SubagentActionFn = Rc<dyn Fn(String, String, &mut Window, &mut App)>;

pub(super) fn render_subagent_footer_actions(
    id: &str,
    task_id: &str,
    p: &Palette,
    on_action: SubagentActionFn,
) -> (AnyElement, AnyElement) {
    let tid_f = task_id.to_owned();
    let tid_s = task_id.to_owned();
    let on_f = on_action.clone();
    let on_s = on_action;
    let followup_btn = h_flex()
        .id(SharedString::from(format!("ai-sub-steer-{id}")))
        .px(px(6.0))
        .py(px(2.0))
        .gap(px(4.0))
        .items_center()
        .rounded(p.radius.sm)
        .border_1()
        .border_color(p.primary.opacity(0.45))
        .bg(p.surface.floating)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .tooltip(|w, cx| {
            gpui_kit::component::tooltip::Tooltip::new(
                rust_i18n::t!("ai.subagent_followup_tooltip").to_string(),
            )
            .build(w, cx)
        })
        .on_click(move |_, w, cx| {
            on_f("followup".to_owned(), tid_f.clone(), w, cx);
        })
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.primary)
                .child(rust_i18n::t!("ai.subagent_followup_btn").to_string()),
        )
        .into_any_element();
    let stop_btn = h_flex()
        .id(SharedString::from(format!("ai-sub-stop-{id}")))
        .px(px(6.0))
        .py(px(2.0))
        .gap(px(4.0))
        .items_center()
        .rounded(p.radius.sm)
        .border_1()
        .border_color(p.danger.opacity(0.45))
        .bg(p.surface.floating)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, w, cx| {
            on_s("stop".to_owned(), tid_s.clone(), w, cx);
        })
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.danger)
                .child(rust_i18n::t!("ai.subagent_stop_btn").to_string()),
        )
        .into_any_element();
    (followup_btn, stop_btn)
}
