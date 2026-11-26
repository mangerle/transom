//! 文件页的单个目录/文件行与右键菜单。

use super::super::*;
use crate::container::ContainerWorkspace;
use crate::theme::tokens::{control, size};
use crate::widgets::{IconButton, IconButtonSize};
use gpui_kit::Stateful;
use gpui_kit::component::menu::PopupMenu;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(super) fn render_entry_row(
    view: &Entity<ContainerWorkspace>,
    entry: &ContainerFileEntry,
    readonly: bool,
    p: &Palette,
) -> AnyElement {
    let entry_path = entry.path.clone();
    let is_dir = entry.kind == ContainerFileKind::Directory;
    let mut row = render_entry_columns(entry, &entry_path, is_dir, p);
    row = if is_dir {
        render_directory_actions(view, row, &entry_path)
    } else {
        render_file_actions(view, row, &entry_path)
    };
    if !readonly {
        row = render_editable_actions(view, row, &entry_path, is_dir);
    }
    let menu_view = view.clone();
    let menu_path = entry_path;
    row.context_menu(move |menu, _, _| {
        append_entry_context_menu(menu, &menu_view, &menu_path, is_dir, readonly)
    })
    .into_any_element()
}

fn render_entry_columns(
    entry: &ContainerFileEntry,
    entry_path: &str,
    is_dir: bool,
    p: &Palette,
) -> Stateful<Div> {
    h_flex()
        .id(ElementId::Name(
            format!("container-file-row-{entry_path}").into(),
        ))
        .h(rems(size::FILE_ROW_HEIGHT))
        .gap_2()
        .px_2()
        .rounded(p.radius.md)
        .items_center()
        .hover(|style| style.bg(p.muted.opacity(0.5)))
        .child(
            AppIconElement::new(if is_dir {
                AppIcon::Ui(UiIcon::FOLDER)
            } else {
                AppIcon::for_file(entry_path)
            })
            .with_rems(rems(control::ICON)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .truncate()
                .child(entry.name.clone()),
        )
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
        )
}

fn render_directory_actions(
    view: &Entity<ContainerWorkspace>,
    row: Stateful<Div>,
    path: &str,
) -> Stateful<Div> {
    let row_view = view.clone();
    let row_path = path.to_string();
    let download_view = view.clone();
    let download_path = path.to_string();
    row.on_click(move |_, _, cx| {
        row_view.update(cx, |workspace, cx| {
            workspace.load_container_dir(row_path.clone(), cx);
        });
    })
    .child(
        IconButton::new(
            ElementId::Name(format!("container-file-download-dir-{path}").into()),
            UiIcon::DOWNLOAD,
            rust_i18n::t!("container.files.download_directory_tooltip").to_string(),
        )
        .size(IconButtonSize::RowAction)
        .render()
        .on_click(move |_, _, cx| {
            download_view.update(cx, |workspace, cx| {
                workspace.download_container_directory(download_path.clone(), cx);
            });
        }),
    )
}

fn render_file_actions(
    view: &Entity<ContainerWorkspace>,
    row: Stateful<Div>,
    path: &str,
) -> Stateful<Div> {
    let row_view = view.clone();
    let row_path = path.to_string();
    let download_view = view.clone();
    let download_path = path.to_string();
    row.on_click(move |_, _, cx| {
        row_view.update(cx, |workspace, cx| {
            workspace.preview_container_file(row_path.clone(), cx);
        });
    })
    .child(
        IconButton::new(
            ElementId::Name(format!("container-file-download-{path}").into()),
            UiIcon::DOWNLOAD,
            rust_i18n::t!("container.files.download").to_string(),
        )
        .size(IconButtonSize::RowAction)
        .render()
        .on_click(move |_, _, cx| {
            download_view.update(cx, |workspace, cx| {
                workspace.download_container_file(download_path.clone(), cx);
            });
        }),
    )
}

fn render_editable_actions(
    view: &Entity<ContainerWorkspace>,
    row: Stateful<Div>,
    path: &str,
    is_dir: bool,
) -> Stateful<Div> {
    let rename_view = view.clone();
    let rename_path = path.to_string();
    let delete_view = view.clone();
    let delete_path = path.to_string();
    row.child(
        IconButton::new(
            ElementId::Name(format!("container-file-rename-{path}").into()),
            UiIcon::EDIT,
            rust_i18n::t!("container.files.rename").to_string(),
        )
        .size(IconButtonSize::RowAction)
        .render()
        .on_click(move |_, window, cx| {
            rename_view.update(cx, |workspace, cx| {
                workspace.begin_container_file_rename(rename_path.clone(), window, cx);
            });
        }),
    )
    .child(
        IconButton::new(
            ElementId::Name(format!("container-file-delete-{path}").into()),
            UiIcon::DELETE,
            rust_i18n::t!("common.delete").to_string(),
        )
        .size(IconButtonSize::RowAction)
        .render()
        .on_click(move |_, _, cx| {
            delete_view.update(cx, |workspace, cx| {
                workspace.request_remove_container_path(delete_path.clone(), is_dir, cx);
            });
        }),
    )
}

fn append_entry_context_menu(
    mut menu: PopupMenu,
    menu_view: &Entity<ContainerWorkspace>,
    menu_path: &str,
    is_dir: bool,
    readonly: bool,
) -> PopupMenu {
    if is_dir {
        let view = menu_view.clone();
        let path = menu_path.to_string();
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
        let path = menu_path.to_string();
        menu = menu.item(crate::widgets::menu_item(
            rust_i18n::t!("container.files.download_directory").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.download_container_directory(path.clone(), cx);
                });
            },
        ));
    } else {
        let view = menu_view.clone();
        let path = menu_path.to_string();
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
        let path = menu_path.to_string();
        menu = menu.item(crate::widgets::menu_item(
            rust_i18n::t!("container.files.download").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.download_container_file(path.clone(), cx);
                });
            },
        ));
    }

    let view = menu_view.clone();
    let path = menu_path.to_string();
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
    let path = menu_path.to_string();
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
    let path = menu_path.to_string();
    menu = menu.item(crate::widgets::menu_item(
        rust_i18n::t!("container.files.chown").to_string(),
        AppIcon::Ui(UiIcon::LOCKED),
        move |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace.begin_container_file_chown(path.clone(), window, cx);
            });
        },
    ));

    if !readonly {
        let view = menu_view.clone();
        let path = menu_path.to_string();
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
        let path = menu_path.to_string();
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
}
