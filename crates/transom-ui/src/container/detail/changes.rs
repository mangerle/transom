//! 容器文件系统变更（Diff）列表视图。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::ClipboardItem;
use transom_core::{ContainerFileSystemChange, ContainerPathChangeKind};

/// 变更页渲染参数：容器标识、变更快照、加载态与搜索输入框/关键词快照。
///
/// 设计原理（Why）：`InputState` 的当前值必须经 `App` 读取，纯渲染函数拿不到
/// `App`，因此由外壳在调用前把关键词一并快照进来；输入框实体仍用于渲染工具栏。
pub(crate) struct ContainerChangesView<'a> {
    pub(super) tab_id: TabId,
    pub(super) container_id: Option<&'a str>,
    pub(super) changes_data: Option<&'a (String, Vec<ContainerFileSystemChange>)>,
    pub(super) loading: bool,
    pub(super) search_input: &'a Entity<InputState>,
    pub(super) search_keyword: &'a str,
}

/// 渲染容器文件变更页签。
///
/// 设计原理（Why）：刷新是实体动作，直接投递变更请求；复制提示写入实体
/// `notice`，由外壳渲染帧的 `flush_container_notice` 统一转成通知，渲染函数
/// 不再需要 `Context<AppShell>`。
pub(crate) fn render_container_changes(
    view: &Entity<ContainerWorkspace>,
    params: ContainerChangesView<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerChangesView {
        tab_id,
        container_id,
        changes_data,
        loading,
        search_input,
        search_keyword,
    } = params;
    let Some(container_id) = container_id else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };

    let search_keyword = search_keyword.trim().to_lowercase();

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

    let refresh_view = view.clone();
    let refresh_btn = Button::new(ElementId::Name(format!("btn-refresh-diff-{tab_id}").into()))
        .ghost()
        .xsmall()
        .icon(IconName::RefreshCw)
        .label(rust_i18n::t!("container.changes.refresh").to_string())
        .on_click(move |_, _, cx| {
            refresh_view.update(cx, |workspace, cx| {
                workspace.refresh_container_changes();
                cx.notify();
            });
        });

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
        let copy_view = view.clone();

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
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_path.clone()));
                    let message = rust_i18n::t!("container.detail.copied").to_string();
                    copy_view.update(cx, |workspace, cx| {
                        workspace.state.notice = Some(message);
                        cx.notify();
                    });
                }),
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
