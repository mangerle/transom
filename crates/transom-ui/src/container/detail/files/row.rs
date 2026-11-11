//! 文件页的单个目录/文件行与右键菜单。

use super::super::*;
use crate::container::ContainerWorkspace;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;
use transom_ui_icons::{AppIcon, render_app_icon};

pub(super) fn render_entry_row(
    view: &Entity<ContainerWorkspace>,
    entry: &ContainerFileEntry,
    readonly: bool,
    p: &Palette,
) -> AnyElement {
    let entry_path = entry.path.clone();
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
        .child(if is_dir {
            render_app_icon(UiIcon::FOLDER.into(), 16.0)
        } else {
            render_app_icon(AppIcon::for_file(&entry_path), 16.0)
        })
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
        let row_view = view.clone();
        let row_path = entry_path.clone();
        row = row
            .on_click(move |_, _, cx| {
                row_view.update(cx, |workspace, cx| {
                    workspace.load_container_dir(row_path.clone(), cx);
                });
            })
            .child(
                Button::new(ElementId::Name(
                    format!("container-file-download-dir-{entry_path}").into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                .accessibility_label(
                    rust_i18n::t!("container.files.download_directory_tooltip").to_string(),
                )
                .tooltip(rust_i18n::t!("container.files.download_directory_tooltip").to_string())
                .on_click({
                    let view = view.clone();
                    let path = entry_path.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.download_container_directory(path.clone(), cx);
                        });
                    }
                }),
            );
    } else {
        let row_view = view.clone();
        let row_path = entry_path.clone();
        row = row
            .on_click(move |_, _, cx| {
                row_view.update(cx, |workspace, cx| {
                    workspace.preview_container_file(row_path.clone(), cx);
                });
            })
            .child(
                Button::new(ElementId::Name(
                    format!("container-file-download-{entry_path}").into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                .accessibility_label("Download file")
                .on_click({
                    let view = view.clone();
                    let path = entry_path.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.download_container_file(path.clone(), cx);
                        });
                    }
                }),
            );
    }
    if !readonly {
        row = row.child(
            Button::new(ElementId::Name(
                format!("container-file-rename-{entry_path}").into(),
            ))
            .ghost()
            .xsmall()
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::EDIT)).small())
            .accessibility_label("Rename file")
            .on_click({
                let view = view.clone();
                let path = entry_path.clone();
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_rename(path.clone(), window, cx);
                    });
                }
            }),
        );
        row = row.child(
            Button::new(ElementId::Name(
                format!("container-file-delete-{entry_path}").into(),
            ))
            .ghost()
            .xsmall()
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
            .accessibility_label("Delete file")
            .on_click({
                let view = view.clone();
                let path = entry_path.clone();
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.request_remove_container_path(path.clone(), is_dir, cx);
                    });
                }
            }),
        );
    }
    // 文件行右键菜单：与 SFTP 面板同样把「打开/预览、下载、改名、删除」收进来，
    // 窄窗下不必再去找行尾那一排小图标。
    let menu_view = view.clone();
    let menu_path = entry_path.clone();
    row.context_menu(move |menu, _, _| {
        let mut menu = menu;
        if is_dir {
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.open").to_string(),
                AppIcon::Ui(UiIcon::FOLDER),
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.load_container_dir(path.clone(), cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.download_directory").to_string(),
                AppIcon::Ui(UiIcon::DOWNLOAD),
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.download_container_directory(path.clone(), cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.copy_to").to_string(),
                AppIcon::Ui(UiIcon::COPY),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_copy(path.clone(), window, cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chmod").to_string(),
                AppIcon::Ui(UiIcon::LOCKED),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_chmod(path.clone(), window, cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chown").to_string(),
                AppIcon::Ui(UiIcon::LOCKED),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_chown(path.clone(), window, cx);
                    });
                },
            ));
        } else {
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.preview").to_string(),
                AppIcon::Ui(UiIcon::DIFF),
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.preview_container_file(path.clone(), cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.download").to_string(),
                AppIcon::Ui(UiIcon::DOWNLOAD),
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.download_container_file(path.clone(), cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.copy_to").to_string(),
                AppIcon::Ui(UiIcon::COPY),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_copy(path.clone(), window, cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chmod").to_string(),
                AppIcon::Ui(UiIcon::LOCKED),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_chmod(path.clone(), window, cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.chown").to_string(),
                AppIcon::Ui(UiIcon::LOCKED),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_chown(path.clone(), window, cx);
                    });
                },
            ));
        }
        if !readonly {
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("container.files.rename").to_string(),
                AppIcon::Ui(UiIcon::EDIT),
                move |_, window, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.begin_container_file_rename(path.clone(), window, cx);
                    });
                },
            ));
            let view = menu_view.clone();
            let path = menu_path.clone();
            menu = menu
                .item(PopupMenuItem::separator())
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("common.delete").to_string(),
                    AppIcon::Ui(UiIcon::DELETE),
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.request_remove_container_path(path.clone(), is_dir, cx);
                        });
                    },
                ));
        }
        menu
    })
    .into_any_element()
}
