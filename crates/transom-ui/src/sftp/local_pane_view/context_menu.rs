//! SFTP 本地文件项右键上下文菜单。
//!
//! 设计原理（Why）：
//! - 实现初衷：将本地文件浏览器列表项中密集的右键上下文菜单项提取为独立函数，
//!   降低主虚拟列表循环与行渲染逻辑的嵌套深度；
//! - 核心优势：集中管理本地文件操作契约（上传、新建、软链接、属性、删除等）。

use crate::sftp::entity::SftpPanelView;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
#[cfg(unix)]
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 本地列表项右键菜单参数。
pub(super) struct LocalContextMenuProps {
    pub(super) tab_id: TabId,
    pub(super) entry_path: PathBuf,
    pub(super) is_parent_entry: bool,
    pub(super) remote_available: bool,
}

/// 构建本地列表项右键菜单。
///
/// 菜单项按动作域拆成具名构造器：父项、文件动作、路径/链接/属性、创建、
/// 改名删除与目录动作各管一段，主函数只决定顺序与条件。
pub(super) fn build_local_context_menu(
    menu: PopupMenu,
    app_entity: Entity<SftpPanelView>,
    props: LocalContextMenuProps,
) -> PopupMenu {
    let ctx = LocalMenuCtx {
        app: app_entity,
        tab_id: props.tab_id,
        entry_path: props.entry_path,
        remote_available: props.remote_available,
    };
    if props.is_parent_entry {
        return build_parent_entry_menu(menu, &ctx);
    }
    let is_file = ctx.entry_path.is_file();
    let mut menu = menu;
    if is_file {
        menu = append_edit_item(menu, &ctx);
    }
    if ctx.remote_available {
        menu = append_upload_item(menu, &ctx);
    }
    menu = append_copy_path_item(menu, &ctx);
    menu = append_symlink_item(menu, &ctx);
    menu = append_properties_item(menu, &ctx);
    #[cfg(unix)]
    {
        menu = append_exec_item(menu, &ctx);
    }
    menu = append_create_items(menu, &ctx);
    menu = append_rename_item(menu, &ctx);
    menu = append_delete_item(menu, &ctx);
    menu = append_open_manager_item(menu, &ctx);
    menu = append_refresh_item(menu, &ctx);
    menu = append_copy_entries_item(menu, &ctx);
    append_paste_item(menu, &ctx)
}

/// 菜单项构造共享的标签、路径与实体句柄。
struct LocalMenuCtx {
    app: Entity<SftpPanelView>,
    tab_id: TabId,
    entry_path: PathBuf,
    remote_available: bool,
}

fn build_parent_entry_menu(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let tab_id = ctx.tab_id;
    let up_app = ctx.app.clone();
    let refresh_app = ctx.app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.go_up").to_string(),
        AppIcon::Ui(UiIcon::UP),
        move |_, window, cx| {
            up_app.update(cx, |this, cx| {
                this.local_go_up(tab_id, window, cx);
            });
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.refresh").to_string(),
        AppIcon::Ui(UiIcon::REFRESH),
        move |_, _, cx| {
            refresh_app.update(cx, |this, cx| {
                this.local_refresh(cx);
            });
        },
    ))
}

fn append_edit_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("common.edit").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                this.local_edit_file(path.clone(), cx);
            });
        },
    ))
}

fn append_upload_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    let tab_id = ctx.tab_id;
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.upload").to_string(),
        AppIcon::Ui(UiIcon::UPLOAD),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                let panel = &mut this.state;
                if panel.local.selection.contains(&path) {
                    panel.set_local_primary(path.clone());
                } else {
                    panel.select_local(path.clone());
                }
                this.queue_upload(tab_id, cx);
            });
        },
    ))
}

fn append_copy_path_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.copy_abs_path").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
            app.update(cx, |this, cx| {
                this.state.set_status(
                    rust_i18n::t!("sftp.menu.local_path_copied", path = path.display()).to_string(),
                );
                cx.notify();
            });
        },
    ))
}

fn append_symlink_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.create_symlink").to_string(),
        AppIcon::Ui(UiIcon::ADD_LINK),
        move |_, window, cx| {
            app.update(cx, |this, cx| {
                this.state.select_local(path.clone());
                this.open_local_symlink_modal(window, cx);
            });
        },
    ))
}

fn append_properties_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.properties").to_string(),
        AppIcon::Custom(CustomIcon::INFO),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                this.show_local_properties(path.clone(), cx);
            });
        },
    ))
}

#[cfg(unix)]
fn append_exec_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.grant_exec").to_string(),
        AppIcon::Domain(DomainIcon::TERMINAL),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                let panel = &mut this.state;
                if panel.local.selection.contains(&path) {
                    panel.set_local_primary(path.clone());
                } else {
                    panel.select_local(path.clone());
                }
                this.grant_local_exec(cx);
            });
        },
    ))
}

fn append_create_items(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let mkdir_app = ctx.app.clone();
    let file_app = ctx.app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.new_local_folder").to_string(),
        AppIcon::Ui(UiIcon::NEW_FOLDER),
        move |_, window, cx| {
            mkdir_app.update(cx, |this, cx| {
                this.open_local_mkdir_modal(window, cx);
            });
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.new_local_file").to_string(),
        AppIcon::Ui(UiIcon::ADD_FILE),
        move |_, window, cx| {
            file_app.update(cx, |this, cx| {
                this.open_local_new_file_modal(window, cx);
            });
        },
    ))
}

fn append_rename_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.rename").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        move |_, window, cx| {
            app.update(cx, |this, cx| {
                this.state.select_local(path.clone());
                this.open_local_rename_modal(window, cx);
            });
        },
    ))
}

fn append_delete_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    let path = ctx.entry_path.clone();
    let tab_id = ctx.tab_id;
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.delete").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                if !this.state.local.selection.contains(&path) {
                    this.state.select_local(path.clone());
                }
                this.local_delete_selection(tab_id, cx);
            });
        },
    ))
}

fn append_open_manager_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.open_file_manager").to_string(),
        AppIcon::Ui(UiIcon::FOLDER),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                this.local_open_file_manager(cx);
            });
        },
    ))
}

fn append_refresh_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.refresh").to_string(),
        AppIcon::Ui(UiIcon::REFRESH),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                this.local_refresh(cx);
            });
        },
    ))
}

fn append_copy_entries_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.copy_entries").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                this.copy_selection(cx);
            });
        },
    ))
}

fn append_paste_item(menu: PopupMenu, ctx: &LocalMenuCtx) -> PopupMenu {
    let app = ctx.app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.paste_here").to_string(),
        AppIcon::Ui(UiIcon::PASTE),
        move |_, _window, cx| {
            app.update(cx, |this, cx| {
                this.paste_clipboard(cx);
            });
        },
    ))
}

/// 本地列空白区域的目录级右键菜单。
///
/// 设计原理（Why）：与远程列同一口径——菜单原先只挂在行上，目录不满一屏时下方空白
/// 点不出任何东西。这里只放与选中条目无关的目录级动作；"上传"需要以条目为源，
/// 仍归行菜单，空白区不放一个必然报错的入口。
pub(super) fn build_local_blank_context_menu(
    menu: PopupMenu,
    app: Entity<SftpPanelView>,
    tab_id: TabId,
) -> PopupMenu {
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.go_up").to_string(),
        AppIcon::Ui(UiIcon::UP),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.local_go_up(tab_id, window, cx);
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
                    this.local_refresh(cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.new_local_folder").to_string(),
        AppIcon::Ui(UiIcon::NEW_FOLDER),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.open_local_mkdir_modal(window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.new_local_file").to_string(),
        AppIcon::Ui(UiIcon::ADD_FILE),
        {
            let app = app.clone();
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.open_local_new_file_modal(window, cx);
                });
            }
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("sftp.menu.paste_here").to_string(),
        AppIcon::Ui(UiIcon::PASTE),
        move |_, _window, cx| {
            app.update(cx, |this, cx| {
                this.paste_clipboard(cx);
            });
        },
    ))
}
