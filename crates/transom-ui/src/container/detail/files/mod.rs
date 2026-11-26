//! 文件页签：目录行、上传与预览入口。
//!
//! 面包屑工具条与目录行分别放在 toolbar.rs / row.rs，本模块负责组装。
//! 渲染只持有工作区实体句柄与状态快照：目录切换、预览、编辑与上传下载都直接
//! `update` 实体，不再需要 `Context<AppShell>`。

mod row;
mod toolbar;

use super::*;
use crate::container::ContainerWorkspace;
use crate::sftp::SftpDragItem;
use crate::theme::tokens::{control, size};
use gpui_kit::component::input::TextareaState;
use gpui_kit::gpui::ExternalPaths;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

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
    view: &Entity<ContainerWorkspace>,
    params: ContainerFilesView<'_>,
    p: &Palette,
) -> AnyElement {
    let current_path = if params.path.is_empty() {
        "/"
    } else {
        params.path
    };
    let list = render_file_list(view, &params, current_path, p);
    let header = render_files_header(view, current_path, params.readonly, p);
    let scroll = render_files_scroll(view, params.readonly, params.scroll, list, p);
    let mut body = v_flex()
        .size_full()
        .overflow_hidden()
        .child(header)
        .child(scroll);
    if let Some(preview) = params.preview {
        body = body.child(super::file_preview::render_file_preview(
            view,
            super::file_preview::FilePreviewView {
                preview,
                preview_editor: params.preview_editor,
                preview_saving: params.preview_saving,
                binary: params.preview_binary,
                readonly: params.readonly,
            },
            p,
        ));
    }
    body.into_any_element()
}

fn render_files_header(
    view: &Entity<ContainerWorkspace>,
    current_path: &str,
    readonly: bool,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_1()
        .px_2()
        .pt_2()
        .pb_1p5()
        .flex_none()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(toolbar::render_toolbar(view, current_path, readonly, p))
        .into_any_element()
}

fn render_file_list(
    view: &Entity<ContainerWorkspace>,
    params: &ContainerFilesView<'_>,
    current_path: &str,
    p: &Palette,
) -> AnyElement {
    let mut ordered: Vec<&ContainerFileEntry> = params.entries.iter().collect();
    ordered.sort_by(|left, right| {
        let left_dir = left.kind == ContainerFileKind::Directory;
        let right_dir = right.kind == ContainerFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut list = v_flex().gap_0p5();
    if params.loading && params.entries.is_empty() {
        list = list.child(file_list_hint(
            rust_i18n::t!("container.loading").to_string(),
            p,
        ));
    }
    list = list.child(render_parent_row(view, current_path, p));
    for entry in ordered {
        list = list.child(row::render_entry_row(view, entry, params.readonly, p));
    }
    if params.entries.is_empty() && !params.loading {
        list = list.child(file_list_hint(
            rust_i18n::t!("container.files.empty").to_string(),
            p,
        ));
    }
    list.into_any_element()
}

fn file_list_hint(text: String, p: &Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}

fn render_parent_row(
    view: &Entity<ContainerWorkspace>,
    current_path: &str,
    p: &Palette,
) -> AnyElement {
    let at_root = current_path == "/";
    let mut row = h_flex()
        .id("container-file-parent-row")
        .h(rems(size::FILE_ROW_HEIGHT))
        .gap_2()
        .px_2()
        .rounded(p.radius.md)
        .items_center()
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::UP))
                .with_rems(rems(control::ICON))
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
        let parent_view = view.clone();
        let parent = parent_path(current_path);
        row = row
            .hover(|style| style.bg(p.muted.opacity(0.5)))
            .on_click(move |_, _, cx| {
                parent_view.update(cx, |workspace, cx| {
                    workspace.load_container_dir(parent.clone(), cx);
                });
            });
    }
    row.into_any_element()
}

fn render_files_scroll(
    view: &Entity<ContainerWorkspace>,
    readonly: bool,
    scroll: &ScrollHandle,
    list: AnyElement,
    p: &Palette,
) -> AnyElement {
    let mut container = div()
        .id("container-files-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .p_2();
    if !readonly {
        let drop_bg = p.sidebar_accent.opacity(0.3);
        let drop_border = p.primary;
        let drop_view = view.clone();
        let sftp_drop_view = view.clone();
        container = container
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
                drop_view.update(cx, |workspace, cx| {
                    for path in dropped {
                        workspace.upload_container_path(path, cx);
                    }
                });
            })
            .on_drop(move |drag: &SftpDragItem, _window, cx| {
                let paths: Vec<std::path::PathBuf> =
                    drag.paths.iter().map(std::path::PathBuf::from).collect();
                sftp_drop_view.update(cx, |workspace, cx| {
                    for path in paths {
                        if path.exists() {
                            workspace.upload_container_path(path, cx);
                        }
                    }
                });
            });
    }
    div()
        .flex_1()
        .min_h_0()
        .child(container.child(list))
        .into_any_element()
}
