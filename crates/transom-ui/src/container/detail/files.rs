//! 文件页签：目录行、上传与预览入口。

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
    pub(super) loading: bool,
    pub(super) rename_from: Option<&'a str>,
    pub(super) rename_input: Option<&'a Entity<InputState>>,
    pub(super) new_dir_open: bool,
    pub(super) new_dir_input: Option<&'a Entity<InputState>>,
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
        loading,
        rename_from: _,
        rename_input: _,
        new_dir_open: _,
        new_dir_input: _,
        scroll,
        readonly,
    } = view;
    let current_path = if path.is_empty() { "/" } else { path };
    let parent = parent_path(current_path);
    let mut crumbs = h_flex().gap_1().items_center().child(
        Button::new("container-file-up")
            .ghost()
            .xsmall()
            .icon(IconName::ArrowLeft)
            .on_click(cx.listener({
                let parent = parent.clone();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, parent.clone());
                    cx.notify();
                }
            })),
    );
    crumbs = crumbs.child(
        div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(current_path.to_string()),
    );
    crumbs = crumbs.child(div().flex_1()).child(
        Button::new("container-file-refresh")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .on_click(cx.listener({
                let path = current_path.to_string();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, path.clone());
                    cx.notify();
                }
            })),
    );
    if !readonly {
        crumbs = crumbs.child(
            Button::new("container-file-upload")
                .ghost()
                .xsmall()
                .icon(IconName::Upload)
                .label(rust_i18n::t!("container.files.upload").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.upload_container_file(tab_id);
                    cx.notify();
                })),
        );
        crumbs = crumbs.child(
            Button::new("container-file-mkdir")
                .ghost()
                .xsmall()
                .icon(IconName::FolderPlus)
                .label(rust_i18n::t!("container.files.new_dir").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_container_mkdir(tab_id, window, cx);
                })),
        );
    }

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
        let entry_path = entry.path.clone();
        let entry_name = entry.name.clone();
        let is_dir = entry.kind == ContainerFileKind::Directory;
        let name = entry.name.clone();
        let mut row = h_flex()
            .id(ElementId::Name(
                format!("container-file-row-{entry_path}").into(),
            ))
            .gap_2()
            .px_2()
            .py_1()
            .rounded(p.radius.md)
            .items_center()
            .hover(|style| style.bg(p.muted.opacity(0.5)))
            .child(
                Icon::new(if is_dir {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small()
                .text_color(p.muted_foreground),
            )
            .child(div().flex_1().min_w_0().text_sm().truncate().child(name))
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .text_color(p.muted_foreground)
                    .child(mode_label(entry.mode)),
            )
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .text_color(p.muted_foreground)
                    .child(entry.modified.map(format_unix).unwrap_or_default()),
            )
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .w(px(64.0))
                    .text_right()
                    .text_color(p.muted_foreground)
                    .child(if is_dir {
                        String::new()
                    } else {
                        crate::format::format_size(entry.size)
                    }),
            );
        if is_dir {
            row = row.on_click(cx.listener({
                let entry_path = entry_path.clone();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, entry_path.clone());
                    cx.notify();
                }
            }));
        } else {
            row = row
                .on_click(cx.listener({
                    let entry_path = entry_path.clone();
                    move |this, _, _, cx| {
                        this.preview_container_file(tab_id, entry_path.clone());
                        cx.notify();
                    }
                }))
                .child(
                    Button::new(ElementId::Name(
                        format!("container-file-download-{entry_path}").into(),
                    ))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Download)
                    .on_click(cx.listener({
                        let entry_path = entry_path.clone();
                        move |this, _, _, cx| {
                            this.download_container_file(tab_id, entry_path.clone());
                            cx.notify();
                        }
                    })),
                );
        }
        if !readonly {
            row = row.child(
                Button::new(ElementId::Name(
                    format!("container-file-rename-{entry_path}").into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::Pen)
                .on_click(cx.listener({
                    let entry_path = entry_path.clone();
                    let entry_name = entry_name.clone();
                    move |this, _, window, cx| {
                        this.begin_container_file_rename(
                            tab_id,
                            entry_path.clone(),
                            entry_name.clone(),
                            window,
                            cx,
                        );
                    }
                })),
            );
            row = row.child(
                Button::new(ElementId::Name(
                    format!("container-file-delete-{entry_path}").into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::Trash)
                .on_click(cx.listener({
                    let entry_path = entry_path.clone();
                    move |this, _, _, cx| {
                        this.request_remove_container_path(tab_id, entry_path.clone(), is_dir, cx);
                    }
                })),
            );
        }
        // 文件行右键菜单：与 SFTP 面板同样把「打开/预览、下载、改名、删除」收进来，
        // 窄窗下不必再去找行尾那一排小图标。
        let menu_app = app.clone();
        let menu_path = entry_path.clone();
        let menu_name = entry_name.clone();
        let row = row
            .context_menu(move |menu, _, _| {
                let mut menu = menu;
                if is_dir {
                    let app = menu_app.clone();
                    let path = menu_path.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.files.open").to_string(),
                        IconName::FolderOpen,
                        move |_, _, cx| {
                            let app = app.clone();
                            let path = path.clone();
                            app.update(cx, |this, cx| {
                                this.load_container_dir(tab_id, path.clone());
                                cx.notify();
                            });
                        },
                    ));
                } else {
                    let app = menu_app.clone();
                    let path = menu_path.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.files.preview").to_string(),
                        IconName::File,
                        move |_, _, cx| {
                            let app = app.clone();
                            let path = path.clone();
                            app.update(cx, |this, cx| {
                                this.preview_container_file(tab_id, path.clone());
                                cx.notify();
                            });
                        },
                    ));
                    let app = menu_app.clone();
                    let path = menu_path.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.files.download").to_string(),
                        IconName::Download,
                        move |_, _, cx| {
                            let app = app.clone();
                            let path = path.clone();
                            app.update(cx, |this, cx| {
                                this.download_container_file(tab_id, path.clone());
                                cx.notify();
                            });
                        },
                    ));
                }
                if !readonly {
                    let app = menu_app.clone();
                    let path = menu_path.clone();
                    let name = menu_name.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.files.rename").to_string(),
                        IconName::Pen,
                        move |_, window, cx| {
                            let app = app.clone();
                            let path = path.clone();
                            let name = name.clone();
                            app.update(cx, |this, cx| {
                                this.begin_container_file_rename(
                                    tab_id,
                                    path.clone(),
                                    name.clone(),
                                    window,
                                    cx,
                                );
                            });
                        },
                    ));
                    let app = menu_app.clone();
                    let path = menu_path.clone();
                    menu = menu.item(PopupMenuItem::separator()).item(crate::widgets::menu_item(
                        rust_i18n::t!("common.delete").to_string(),
                        IconName::Trash,
                        move |_, _, cx| {
                            let app = app.clone();
                            let path = path.clone();
                            app.update(cx, |this, cx| {
                                this.request_remove_container_path(
                                    tab_id,
                                    path.clone(),
                                    is_dir,
                                    cx,
                                );
                            });
                        },
                    ));
                }
                menu
            })
            .into_any_element();
        list = list.child(row);
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
            tab_id,
            preview,
            preview_editor,
            preview_saving,
            readonly,
            p,
            cx,
        ));
    }
    body.into_any_element()
}
