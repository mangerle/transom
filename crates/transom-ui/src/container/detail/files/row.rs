//! 文件页的单个目录/文件行与右键菜单。

use super::super::*;

pub(super) fn render_entry_row(
    tab_id: TabId,
    entry: &ContainerFileEntry,
    readonly: bool,
    app: &Entity<AppShell>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
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
        row = row
            .on_click(cx.listener({
                let entry_path = entry_path.clone();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, entry_path.clone());
                    cx.notify();
                }
            }))
            .child(
                Button::new(ElementId::Name(
                    format!("container-file-download-dir-{entry_path}").into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::Download)
                .tooltip(rust_i18n::t!("container.files.download_directory_tooltip").to_string())
                .on_click(cx.listener({
                    let entry_path = entry_path.clone();
                    move |this, _, _, cx| {
                        this.download_container_directory(tab_id, entry_path.clone());
                        cx.notify();
                    }
                })),
            );
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
    row.context_menu(move |menu, _, _| {
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
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.download_directory").to_string(),
                IconName::Download,
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.download_container_directory(tab_id, path.clone());
                        cx.notify();
                    });
                },
            ));
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.copy_to").to_string(),
                IconName::Copy,
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.begin_container_file_copy(tab_id, path.clone(), window, cx);
                        cx.notify();
                    });
                },
            ));
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chmod").to_string(),
                IconName::Lock,
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.begin_container_file_chmod(tab_id, path.clone(), window, cx);
                        cx.notify();
                    });
                },
            ));
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chown").to_string(),
                IconName::KeyRound,
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.begin_container_file_chown(tab_id, path.clone(), window, cx);
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
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.copy_to").to_string(),
                IconName::Copy,
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.begin_container_file_copy(tab_id, path.clone(), window, cx);
                        cx.notify();
                    });
                },
            ));
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chmod").to_string(),
                IconName::Lock,
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.begin_container_file_chmod(tab_id, path.clone(), window, cx);
                        cx.notify();
                    });
                },
            ));
            let app = menu_app.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chown").to_string(),
                IconName::KeyRound,
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.begin_container_file_chown(tab_id, path.clone(), window, cx);
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
            menu = menu
                .item(PopupMenuItem::separator())
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("common.delete").to_string(),
                    IconName::Trash,
                    move |_, _, cx| {
                        let app = app.clone();
                        let path = path.clone();
                        app.update(cx, |this, cx| {
                            this.request_remove_container_path(tab_id, path.clone(), is_dir, cx);
                        });
                    },
                ));
        }
        menu
    })
    .into_any_element()
}
