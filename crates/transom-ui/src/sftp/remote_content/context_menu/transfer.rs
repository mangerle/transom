//! 远端条目的下载、编辑、归档与跨列复制菜单构造。

use crate::sftp::SftpRemotePaneRef;
use crate::sftp::entity::SftpPanelView;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, DomainIcon, UiIcon};

/// 判断指定路径是否属于受支持的常见压缩包格式。
fn is_archive_name(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar")
        || lower.ends_with(".zip")
}

/// 剥离归档文件名末尾的压缩后缀，提取基础名称作为默认解压目录名。
fn strip_archive_suffix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") {
        name[..name.len() - 7].to_string()
    } else if lower.ends_with(".tgz") || lower.ends_with(".tar") || lower.ends_with(".zip") {
        name[..name.len() - 4].to_string()
    } else {
        name.to_string()
    }
}

pub(super) struct RemoteMenuItemCtx<'a> {
    pub(super) target: SftpRemotePaneRef,
    pub(super) entry_path: &'a str,
    pub(super) is_dir: bool,
    pub(super) has_main_remote: bool,
}

/// 构建远端条目的下载菜单项。
fn append_download_item(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    target: SftpRemotePaneRef,
    entry_path: &str,
) -> PopupMenu {
    let path = entry_path.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.download").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, path.clone(), cx);
                    this.download_pane(target, cx);
                });
            }
        },
    ))
}

/// 构建远端文件的内置编辑菜单项。
fn append_edit_item(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    target: SftpRemotePaneRef,
    entry_path: &str,
) -> PopupMenu {
    let path = entry_path.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.builtin_edit").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, path.clone(), cx);
                    this.edit_remote_file(target, path.clone(), window, cx);
                });
            }
        },
    ))
}

/// 构建归档文件的“原地解压”和“解压到新目录”菜单项。
fn append_extract_items(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    target: SftpRemotePaneRef,
    entry_path: &str,
) -> PopupMenu {
    let here_path = entry_path.to_string();
    let new_path = entry_path.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.extract_here").to_string(),
        AppIcon::Ui(UiIcon::PATCH),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.extract_here(target, here_path.clone(), cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.extract_to_new_folder").to_string(),
        AppIcon::Ui(UiIcon::FOLDER),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    let file_name = new_path
                        .rsplit_once('/')
                        .map(|(_, name)| name)
                        .unwrap_or(&new_path);
                    let default_dir = strip_archive_suffix(file_name);
                    this.open_extract_modal(target, new_path.clone(), default_dir, window, cx);
                });
            }
        },
    ))
}

/// 构建单个压缩格式菜单项；选中项优先作为压缩源。
fn append_archive_format_item(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    target: SftpRemotePaneRef,
    entry_path: &str,
    extension: &'static str,
    label_key: &'static str,
) -> PopupMenu {
    let selected_path = entry_path.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!(label_key).to_string(),
        AppIcon::Ui(UiIcon::PATCH),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    let mut sources = Vec::new();
                    if let Some(pane) = this.state.remote_pane(target)
                        && pane.selection.contains(&selected_path)
                    {
                        sources = pane.selection.iter().cloned().collect();
                    }
                    if sources.is_empty() {
                        sources.push(selected_path.clone());
                    }
                    let default_name = archive_default_name(&sources, extension);
                    this.open_archive_modal(target, sources, default_name, window, cx);
                });
            }
        },
    ))
}

/// 构建并列远端复制到主远端的菜单项。
fn append_copy_to_main_item(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    target: SftpRemotePaneRef,
    entry_path: &str,
    is_dir: bool,
) -> PopupMenu {
    let path = entry_path.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.copy_to_main_remote").to_string(),
        AppIcon::Domain(DomainIcon::FORWARD),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.copy_entry_to_main(target, path.clone(), is_dir, cx);
                });
            }
        },
    ))
}

/// 组合下载、编辑、归档和跨列复制菜单项。
pub(super) fn append_archive_and_transfer_items(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    ctx: RemoteMenuItemCtx<'_>,
) -> PopupMenu {
    let RemoteMenuItemCtx {
        target,
        entry_path,
        is_dir,
        has_main_remote,
    } = ctx;
    let mut menu = append_download_item(menu, app, target, entry_path);
    if !is_dir {
        menu = append_edit_item(menu, app, target, entry_path);
    }
    if !is_dir && is_archive_name(entry_path) {
        menu = append_extract_items(menu, app, target, entry_path);
    }
    menu = append_archive_format_item(
        menu,
        app,
        target,
        entry_path,
        "tar.gz",
        "sftp.menu.compress_tar_gz",
    );
    menu = append_archive_format_item(
        menu,
        app,
        target,
        entry_path,
        "zip",
        "sftp.menu.compress_zip",
    );
    if target != SftpRemotePaneRef::Main && has_main_remote {
        menu = append_copy_to_main_item(menu, app, target, entry_path, is_dir);
    }
    menu
}

/// 按选中项生成归档默认名：单项沿用其文件名，多项使用通用名。
fn archive_default_name(sources: &[String], extension: &str) -> String {
    if sources.len() == 1 {
        let item_name = sources[0]
            .trim_end_matches('/')
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(&sources[0]);
        format!("{item_name}.{extension}")
    } else {
        format!("archive.{extension}")
    }
}
