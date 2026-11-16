//! 远端文件项与空白区的右键菜单：主远端与并列远端共用同一套菜单。
//!
//! 设计原理（Why）：
//! - 实现初衷：并列远端列此前只有下载与复制几个菜单项，主远端的编辑、压缩、
//!   解压、软链、属性、授权执行等能力无法在挂载列使用；两套菜单还各自维护
//!   选中与动作口径；
//! - 核心优势：菜单只认 `tab_id + target`，所有动作经统一动作入口按目标列
//!   路由，新增一列能力时菜单不需要再写第二遍；
//! - 代价与局限：主远端与并列列的动作实现仍在入口后面分支（过渡层），
//!   但调用方看到的菜单项与文案已经完全一致。

use crate::sftp::SftpRemotePaneRef;
use crate::sftp::entity::SftpPanelView;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use transom_core::TabId;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

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

/// 远端右键菜单构建所需的参数上下文对象。
pub(crate) struct RemoteContextMenuProps {
    pub(crate) app: Entity<SftpPanelView>,
    pub(crate) tab_id: TabId,
    pub(crate) target: SftpRemotePaneRef,
    pub(crate) entry_path: String,
    pub(crate) is_dir: bool,
    pub(crate) is_parent_entry: bool,
    /// 标签页是否有主远端连接；没有主远端时不显示“复制到主远端”。
    pub(crate) has_main_remote: bool,
}

/// 构建远端文件/目录右键菜单。
pub(crate) fn build_remote_context_menu(
    menu: PopupMenu,
    props: RemoteContextMenuProps,
) -> PopupMenu {
    let RemoteContextMenuProps {
        app,
        tab_id,
        target,
        entry_path,
        is_dir,
        is_parent_entry,
        has_main_remote,
    } = props;

    if is_parent_entry {
        return menu
            .item(crate::widgets::menu_item(
                rust_i18n::t!("sftp.menu.go_up").to_string(),
                AppIcon::Ui(UiIcon::UP),
                {
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.go_up_pane(tab_id, target, window, cx);
                        });
                    }
                },
            ))
            .item(crate::widgets::menu_item(
                rust_i18n::t!("sftp.refresh").to_string(),
                AppIcon::Ui(UiIcon::REFRESH),
                {
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.refresh_pane(tab_id, target, cx);
                        });
                    }
                },
            ));
    }

    let m = append_archive_and_transfer_items(
        menu,
        &app,
        RemoteMenuItemCtx {
            target,
            entry_path: &entry_path,
            is_dir,
            has_main_remote,
        },
    );
    append_file_action_items(m, &app, tab_id, target, &entry_path)
}

struct RemoteMenuItemCtx<'a> {
    target: SftpRemotePaneRef,
    entry_path: &'a str,
    is_dir: bool,
    has_main_remote: bool,
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
fn append_archive_and_transfer_items(
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

fn append_file_action_items(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    tab_id: TabId,
    target: SftpRemotePaneRef,
    entry_path: &str,
) -> PopupMenu {
    let p_cp = entry_path.to_string();
    let p_sym = entry_path.to_string();
    let p_prop = entry_path.to_string();
    let p_exec = entry_path.to_string();
    let p_chmod = entry_path.to_string();
    let p_ren = entry_path.to_string();
    let p_del = entry_path.to_string();

    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.copy_abs_path").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        {
            let app = app.clone();
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(p_cp.clone()));
                app.update(cx, |this, cx| {
                    this.state.set_status(
                        rust_i18n::t!("sftp.menu.remote_path_copied", path = p_cp).to_string(),
                    );
                    cx.notify();
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.create_symlink").to_string(),
        AppIcon::Ui(UiIcon::ADD_LINK),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, p_sym.clone(), cx);
                    this.open_symlink_modal(target, window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.properties").to_string(),
        AppIcon::Custom(CustomIcon::INFO),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.request_remote_properties(target, p_prop.clone(), cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.grant_exec").to_string(),
        AppIcon::Domain(DomainIcon::TERMINAL),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, p_exec.clone(), cx);
                    this.grant_remote_exec(target, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.chmod").to_string(),
        AppIcon::Ui(UiIcon::SETTINGS),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, p_chmod.clone(), cx);
                    this.open_chmod_modal(target, window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.new_remote_folder").to_string(),
        AppIcon::Ui(UiIcon::NEW_FOLDER),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.open_mkdir_modal_for(target, window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.rename").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, p_ren.clone(), cx);
                    this.open_rename_modal_for(target, window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.delete").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.select_entry_for_menu(target, p_del.clone(), cx);
                    this.confirm_remote_delete(target, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.refresh").to_string(),
        AppIcon::Ui(UiIcon::REFRESH),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.refresh_pane(tab_id, target, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.copy_entries").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.activate_pane(target, cx);
                    this.copy_selection(cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.paste_here").to_string(),
        AppIcon::Ui(UiIcon::PASTE),
        {
            let app = app.clone();
            move |_, _window, cx| {
                app.update(cx, |this, cx| {
                    this.activate_pane(target, cx);
                    this.paste_clipboard(cx);
                });
            }
        },
    ))
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

/// 远端列空白区域（未命中任何条目）的目录级右键菜单。
pub(crate) fn build_remote_blank_context_menu(
    menu: PopupMenu,
    app: Entity<SftpPanelView>,
    tab_id: TabId,
    target: SftpRemotePaneRef,
) -> PopupMenu {
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.go_up").to_string(),
        AppIcon::Ui(UiIcon::UP),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.go_up_pane(tab_id, target, window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.refresh").to_string(),
        AppIcon::Ui(UiIcon::REFRESH),
        {
            let app = app.clone();
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.refresh_pane(tab_id, target, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.new_remote_folder").to_string(),
        AppIcon::Ui(UiIcon::NEW_FOLDER),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.open_mkdir_modal_for(target, window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.paste_here").to_string(),
        AppIcon::Ui(UiIcon::PASTE),
        move |_, _window, cx| {
            app.update(cx, |this, cx| {
                this.activate_pane(target, cx);
                this.paste_clipboard(cx);
            });
        },
    ))
}
