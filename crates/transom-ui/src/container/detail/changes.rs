//! 容器文件系统变更（Diff）列表视图。

use super::*;
use gpui_kit::ClipboardItem;
use transom_core::{
    ContainerCommand, ContainerFileSystemChange, ContainerPathChangeKind, ContainerRequestId,
};

/// 渲染容器文件变更页签。
pub(crate) fn render_container_changes(
    tab_id: TabId,
    container_id: Option<&str>,
    changes_data: Option<&(String, Vec<ContainerFileSystemChange>)>,
    loading: bool,
    search_input: &Entity<InputState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(container_id) = container_id else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };

    let search_keyword = search_input.read(cx).value().trim().to_lowercase();

    let all_changes = changes_data
        .filter(|(id, _)| id == container_id)
        .map(|(_, list)| list.as_slice())
        .unwrap_or(&[]);

    let filtered: Vec<&ContainerFileSystemChange> = all_changes
        .iter()
        .filter(|change| {
            if search_keyword.is_empty() {
                true
            } else {
                change.path.to_lowercase().contains(&search_keyword)
            }
        })
        .collect();

    // 统计各类变更数
    let mut added_count = 0;
    let mut modified_count = 0;
    let mut deleted_count = 0;
    for change in all_changes {
        match change.kind {
            ContainerPathChangeKind::Added => added_count += 1,
            ContainerPathChangeKind::Modified => modified_count += 1,
            ContainerPathChangeKind::Deleted => deleted_count += 1,
        }
    }

    let refresh_cid = container_id.to_string();
    let refresh_btn = Button::new(ElementId::Name(format!("btn-refresh-diff-{tab_id}").into()))
        .ghost()
        .xsmall()
        .icon(IconName::RefreshCw)
        .label(rust_i18n::t!("container.changes.refresh").to_string())
        .on_click(cx.listener({
            let cid = refresh_cid.clone();
            move |this, _, _, cx| {
                let request_id = ContainerRequestId::new();
                if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                    workspace.changes_loading = true;
                    workspace.changes_request = Some(request_id);
                }
                this.state.send_command(ContainerCommand::DiffContainer {
                    tab_id,
                    request_id,
                    container: cid.clone(),
                });
                cx.notify();
            }
        }));

    let stats_badges = h_flex()
        .gap_1p5()
        .items_center()
        .child(change_badge("A", added_count, p.success, p))
        .child(change_badge("C", modified_count, p.warning, p))
        .child(change_badge("D", deleted_count, p.danger, p));

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
                .child(stats_badges),
        )
        .child(refresh_btn);

    if loading && all_changes.is_empty() {
        return v_flex()
            .child(toolbar)
            .child(centered_hint(
                rust_i18n::t!("container.changes.loading").to_string(),
                p,
            ))
            .into_any_element();
    }

    if all_changes.is_empty() {
        return v_flex()
            .child(toolbar)
            .child(centered_hint(
                rust_i18n::t!("container.changes.no_changes").to_string(),
                p,
            ))
            .into_any_element();
    }

    if filtered.is_empty() {
        return v_flex()
            .child(toolbar)
            .child(centered_hint(
                rust_i18n::t!("container.changes.no_results").to_string(),
                p,
            ))
            .into_any_element();
    }

    let mut rows = v_flex().gap_0p5();
    for change in filtered {
        let (badge_text, badge_color) = match change.kind {
            ContainerPathChangeKind::Added => ("A", p.success),
            ContainerPathChangeKind::Modified => ("C", p.warning),
            ContainerPathChangeKind::Deleted => ("D", p.danger),
        };

        let path_text = change.path.clone();
        let copy_path = path_text.clone();

        let row = h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .px_2()
            .rounded(p.radius.sm)
            .hover(|s| s.bg(p.muted.opacity(0.2)))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .w_5()
                            .h_5()
                            .flex_none()
                            .rounded(p.radius.sm)
                            .bg(badge_color.opacity(0.18))
                            .text_color(badge_color)
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .items_center()
                            .justify_center()
                            .child(badge_text),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(p.mono_font_family.clone())
                            .text_size(p.mono_font_size)
                            .truncate()
                            .child(path_text),
                    ),
            )
            .child(
                Button::new(ElementId::Name(
                    format!("btn-copy-diff-path-{}", change.path).into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .tooltip(rust_i18n::t!("container.detail.copy_path").to_string())
                .on_click(cx.listener({
                    let path = copy_path.clone();
                    move |this, _, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        this.show_toast(
                            rust_i18n::t!("container.detail.copied").to_string(),
                            true,
                            window,
                            cx,
                        );
                    }
                })),
            );

        rows = rows.child(row);
    }

    scroll_body(v_flex().child(toolbar).child(rows), p).into_any_element()
}

fn change_badge(letter: &'static str, count: usize, color: Hsla, p: &Palette) -> AnyElement {
    h_flex()
        .gap_1()
        .items_center()
        .px_1p5()
        .py_0p5()
        .rounded(p.radius.sm)
        .bg(color.opacity(0.12))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(letter),
        )
        .child(
            div()
                .text_xs()
                .font_family(p.mono_font_family.clone())
                .text_color(p.foreground.opacity(0.8))
                .child(count.to_string()),
        )
        .into_any_element()
}
