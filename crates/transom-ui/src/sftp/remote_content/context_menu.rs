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

use crate::sftp::entity::SftpPanelView;
use crate::sftp::{SftpPanelEvent, SftpRemotePaneRef, SftpTerminalDirectoryPath, parent_remote};
mod transfer;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use transom_core::{ConnectionId, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 远端右键菜单构建所需的参数上下文对象。
pub(crate) struct RemoteContextMenuProps {
    pub(crate) app: Entity<SftpPanelView>,
    pub(crate) tab_id: TabId,
    pub(crate) target: SftpRemotePaneRef,
    pub(crate) connection_id: ConnectionId,
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
        connection_id,
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

    let m = transfer::append_archive_and_transfer_items(
        menu,
        &app,
        transfer::RemoteMenuItemCtx {
            target,
            entry_path: &entry_path,
            is_dir,
            has_main_remote,
        },
    );
    let terminal_path = terminal_target_path(&entry_path, is_dir);
    let m = append_terminal_directory_item(m, &app, connection_id, terminal_path);
    append_file_action_items(m, &app, tab_id, target, &entry_path)
}

/// 目录项进入自身，文件项进入其远端父目录。
fn terminal_target_path(path: &str, is_dir: bool) -> String {
    if is_dir {
        path.to_string()
    } else {
        parent_remote(path)
    }
}

fn append_terminal_directory_item(
    menu: PopupMenu,
    app: &Entity<SftpPanelView>,
    connection_id: ConnectionId,
    target_path: String,
) -> PopupMenu {
    let app = app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.change_terminal_directory").to_string(),
        AppIcon::Domain(DomainIcon::TERMINAL),
        move |_, _, cx| {
            app.update(cx, |_, cx| {
                cx.emit(SftpPanelEvent::ChangeTerminalDirectory {
                    connection_id,
                    path: SftpTerminalDirectoryPath::Remote(target_path.clone()),
                });
            });
        },
    ))
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

/// 远端列空白区域右键菜单的输入快照。
pub(crate) struct RemoteBlankContextMenuProps {
    pub(crate) app: Entity<SftpPanelView>,
    pub(crate) tab_id: TabId,
    pub(crate) target: SftpRemotePaneRef,
    pub(crate) connection_id: ConnectionId,
    pub(crate) current_path: String,
}

/// 远端列空白区域（未命中任何条目）的目录级右键菜单。
pub(crate) fn build_remote_blank_context_menu(
    menu: PopupMenu,
    props: RemoteBlankContextMenuProps,
) -> PopupMenu {
    let RemoteBlankContextMenuProps {
        app,
        tab_id,
        target,
        connection_id,
        current_path,
    } = props;
    let terminal_app = app.clone();
    let mut menu = menu
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
        ));
    menu = menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.change_terminal_directory").to_string(),
        AppIcon::Domain(DomainIcon::TERMINAL),
        move |_, _, cx| {
            terminal_app.update(cx, |_, cx| {
                cx.emit(SftpPanelEvent::ChangeTerminalDirectory {
                    connection_id,
                    path: SftpTerminalDirectoryPath::Remote(current_path.clone()),
                });
            });
        },
    ));
    menu
}

#[cfg(test)]
mod tests {
    use super::terminal_target_path;

    #[test]
    fn terminal_target_uses_directory_or_remote_parent() {
        assert_eq!(terminal_target_path("/srv/app", true), "/srv/app");
        assert_eq!(terminal_target_path("/srv/app/file.txt", false), "/srv/app");
        assert_eq!(terminal_target_path("/", false), "/");
    }
}
