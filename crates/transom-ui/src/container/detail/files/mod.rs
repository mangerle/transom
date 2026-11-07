//! 文件页签：目录行、上传与预览入口。
//!
//! 面包屑工具条与目录行分别放在 toolbar.rs / row.rs，本模块负责组装。

mod row;
mod toolbar;

use super::*;
use crate::sftp::SftpDragItem;
use gpui_kit::component::input::TextareaState;
use gpui_kit::gpui::ExternalPaths;

/// 文件页签的渲染参数：字段多且都是借用，收敛为结构体避免长参数列表。
pub(crate) struct ContainerFilesView<'a> {
    pub(super) path: &'a str,
    pub(super) entries: &'a [ContainerFileEntry],
    pub(super) preview: Option<&'a (String, String, bool)>,
    pub(super) preview_editor: Option<&'a Entity<TextareaState>>,
    pub(super) preview_saving: bool,
    pub(super) preview_binary: bool,
    pub(super) loading: bool,
    pub(super) scroll: &'a ScrollHandle,
    pub(super) readonly: bool,
}

pub(crate) fn render_files(
    tab_id: TabId,
    view: ContainerFilesView<'_>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let ContainerFilesView {
        path,
        entries,
        preview,
        preview_editor,
        preview_saving,
        preview_binary,
        loading,
        scroll,
        readonly,
    } = view;
    let current_path = if path.is_empty() { "/" } else { path };
    let parent = parent_path(current_path);
    let crumbs = toolbar::render_toolbar(tab_id, current_path, readonly, p, cx);

    // 目录优先、再按名称：归档接口按 tar 顺序返回，用户习惯是先看目录。
    let mut ordered: Vec<&ContainerFileEntry> = entries.iter().collect();
    ordered.sort_by(|left, right| {
        let left_dir = left.kind == ContainerFileKind::Directory;
        let right_dir = right.kind == ContainerFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.cmp(&right.name))
    });

    let app = cx.entity().clone();
    let mut list = v_flex().gap_0p5();
    if loading && entries.is_empty() {
        list = list.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.loading").to_string()),
        );
    }
    // 目录列表顶部的 `..`：根目录也占同一行（置灰不可点），否则进出根目录时
    // 整张列表会上下跳一行，看起来就是闪烁。
    let at_root = current_path == "/";
    let mut parent_row = h_flex()
        .id("container-file-parent-row")
        .gap_2()
        .px_2()
        .py_1()
        .rounded(p.radius.md)
        .items_center()
        .child(
            Icon::new(IconName::CornerLeftUp)
                .small()
                .text_color(p.muted_foreground),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(if at_root {
                    p.muted_foreground
                } else {
                    p.foreground
                })
                .child(".."),
        );
    if !at_root {
        let parent = parent.clone();
        parent_row = parent_row
            .hover(|style| style.bg(p.muted.opacity(0.5)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.load_container_dir(tab_id, parent.clone());
                cx.notify();
            }));
    }
    list = list.child(parent_row);
    for entry in ordered {
        list = list.child(row::render_entry_row(tab_id, entry, readonly, &app, p, cx));
    }
    if entries.is_empty() && !loading {
        list = list.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.files.empty").to_string()),
        );
    }

    // 头部固定、列表撑满剩余高度：原先把滚动区写死 280px，下面留一大块空白，
    // 滚到底时最后一行还贴着硬边界被截断。
    let header = v_flex()
        .gap_1()
        .px_2()
        .pt_2()
        .pb_1p5()
        .flex_none()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(crumbs);

    let drop_bg = p.sidebar_accent.opacity(0.3);
    let drop_border = p.primary;
    let app_drop = app.clone();
    let app_sftp_drop = app.clone();

    let mut scroll_container = div()
        // 滚动容器用稳定 id，切目录只复位偏移不重建整块列表；换 id 会让
        // GPUI 丢弃该元素并在同一帧重新挂载，肉眼就是一次闪烁。
        .id("container-files-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .p_2();

    if !readonly {
        scroll_container = scroll_container
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.bg(drop_bg).border_2().border_color(drop_border)
            })
            .drag_over::<SftpDragItem>(move |style, _, _, _| {
                style.bg(drop_bg).border_2().border_color(drop_border)
            })
            .on_drop(move |paths: &ExternalPaths, _window, cx| {
                let dropped = paths.paths().to_vec();
                if dropped.is_empty() {
                    return;
                }
                app_drop.update(cx, |this, _| {
                    for path in dropped {
                        this.upload_container_path(tab_id, path);
                    }
                });
            })
            .on_drop(move |drag: &SftpDragItem, _window, cx| {
                let paths: Vec<std::path::PathBuf> =
                    drag.paths.iter().map(std::path::PathBuf::from).collect();
                app_sftp_drop.update(cx, |this, _| {
                    for path in paths {
                        if path.exists() {
                            this.upload_container_path(tab_id, path);
                        }
                    }
                });
            });
    }

    let mut body = v_flex()
        .size_full()
        .overflow_hidden()
        .child(header)
        .child(scroll_container.child(list));
    if let Some(preview) = preview {
        body = body.child(super::file_preview::render_file_preview(
            super::file_preview::FilePreviewView {
                tab_id,
                preview,
                preview_editor,
                preview_saving,
                binary: preview_binary,
                readonly,
            },
            p,
            cx,
        ));
    }
    body.into_any_element()
}
