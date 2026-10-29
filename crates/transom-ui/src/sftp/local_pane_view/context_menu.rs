//! SFTP 本地文件项右键上下文菜单。
//!
//! 设计原理（Why）：
//! - 实现初衷：将本地文件浏览器列表项中密集的右键上下文菜单项提取为独立函数，
//!   降低主虚拟列表循环与行渲染逻辑的嵌套深度；
//! - 核心优势：集中管理本地文件操作契约（上传、新建、软链接、属性、删除等）。

use crate::app::AppShell;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;

/// 本地列表项右键菜单参数。
pub(super) struct LocalContextMenuProps {
    pub(super) tab_id: TabId,
    pub(super) entry_path: PathBuf,
    pub(super) is_parent_entry: bool,
    pub(super) remote_available: bool,
}

/// 构建本地列表项右键菜单。
pub(super) fn build_local_context_menu(
    menu: PopupMenu,
    app_entity: Entity<AppShell>,
    props: LocalContextMenuProps,
) -> PopupMenu {
    let tab_id = props.tab_id;
    let is_parent_entry = props.is_parent_entry;
    let remote_available = props.remote_available;
    let entry_path = props.entry_path;

    if is_parent_entry {
        return menu
            .item(
                PopupMenuItem::new(rust_i18n::t!("sftp.menu.go_up").to_string())
                    .icon(IconName::ArrowUp)
                    .on_click({
                        let app = app_entity.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.sftp_local_go_up(tab_id, window, cx);
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("sftp.refresh").to_string())
                    .icon(IconName::RefreshCw)
                    .on_click({
                        let app = app_entity;
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.sftp_local_refresh(tab_id, cx);
                            });
                        }
                    }),
            );
    }

    let p_up = entry_path.clone();
    let p_edit = entry_path.clone();
    let is_file = entry_path.is_file();
    let p_cp = entry_path.clone();
    let p_sym = entry_path.clone();
    let p_prop = entry_path.clone();
    #[cfg(unix)]
    let p_exec = entry_path.clone();
    let p_ren = entry_path.clone();
    let p_del = entry_path;
    let mut menu = menu;

    if is_file {
        menu = menu.item(
            PopupMenuItem::new(rust_i18n::t!("common.edit").to_string())
                .icon(IconName::FilePen)
                .on_click({
                    let app = app_entity.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.sftp_edit_local_file(tab_id, p_edit.clone(), cx);
                        });
                    }
                }),
        );
    }

    if remote_available {
        menu = menu.item(
            PopupMenuItem::new(rust_i18n::t!("sftp.upload").to_string())
                .icon(IconName::Upload)
                .on_click({
                    let app = app_entity.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            if let Some(panel) =
                                this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            {
                                if panel.local.selection.contains(&p_up) {
                                    panel.set_local_primary(p_up.clone());
                                } else {
                                    panel.select_local(p_up.clone());
                                }
                            }
                            this.sftp_queue_upload(tab_id, cx);
                        });
                    }
                }),
        );
    }

    menu = menu
        .item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.copy_abs_path").to_string())
                .icon(IconName::Copy)
                .on_click({
                    let app = app_entity.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            p_cp.display().to_string(),
                        ));
                        app.update(cx, |this, cx| {
                            if let Some(panel) =
                                this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            {
                                panel.set_status(
                                    rust_i18n::t!(
                                        "sftp.menu.local_path_copied",
                                        path = p_cp.display()
                                    )
                                    .to_string(),
                                );
                            }
                            cx.notify();
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.create_symlink").to_string())
                .icon(IconName::Link2)
                .on_click({
                    let app = app_entity.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            if let Some(panel) =
                                this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            {
                                panel.select_local(p_sym.clone());
                            }
                            this.sftp_open_symlink_modal(tab_id, false, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.properties").to_string())
                .icon(IconName::Info)
                .on_click({
                    let app = app_entity.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.sftp_show_local_properties(tab_id, p_prop.clone(), cx);
                        });
                    }
                }),
        );

    // 执行位是 Unix 语义：Windows 下整个菜单项不出现，而不是点击后报"不支持"。
    #[cfg(unix)]
    {
        menu = menu.item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.grant_exec").to_string())
                .icon(IconName::Terminal)
                .on_click({
                    let app = app_entity.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            if let Some(panel) =
                                this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            {
                                if panel.local.selection.contains(&p_exec) {
                                    panel.set_local_primary(p_exec.clone());
                                } else {
                                    panel.select_local(p_exec.clone());
                                }
                            }
                            this.sftp_grant_local_exec(tab_id, cx);
                        });
                    }
                }),
        );
    }

    menu.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.new_local_folder").to_string())
            .icon(IconName::FolderPlus)
            .on_click({
                let app = app_entity.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_mkdir_modal(tab_id, false, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.new_local_file").to_string())
            .icon(IconName::FilePlus)
            .on_click({
                let app = app_entity.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_local_new_file_modal(tab_id, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.rename").to_string())
            .icon(IconName::Pencil)
            .on_click({
                let app = app_entity.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
                            panel.select_local(p_ren.clone());
                        }
                        this.sftp_open_rename_modal(tab_id, false, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.delete").to_string())
            .icon(IconName::Trash)
            .on_click({
                let app = app_entity.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            && !panel.local.selection.contains(&p_del)
                        {
                            panel.select_local(p_del.clone());
                        }
                        this.sftp_local_delete_selection(tab_id, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.open_file_manager").to_string())
            .icon(IconName::FolderOpen)
            .on_click({
                let app = app_entity.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_local_file_manager(tab_id, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.refresh").to_string())
            .icon(IconName::RefreshCw)
            .on_click({
                let app = app_entity.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_local_refresh(tab_id, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.copy_entries").to_string())
            .icon(IconName::Copy)
            .on_click({
                let app = app_entity.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_copy_selection(tab_id, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.paste_here").to_string())
            .icon(IconName::Clipboard)
            .on_click({
                let app = app_entity;
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_paste_clipboard(tab_id, window, cx);
                    });
                }
            }),
    )
}

/// 本地列空白区域的目录级右键菜单。
///
/// 设计原理（Why）：与远程列同一口径——菜单原先只挂在行上，目录不满一屏时下方空白
/// 点不出任何东西。这里只放与选中条目无关的目录级动作；"上传"需要以条目为源，
/// 仍归行菜单，空白区不放一个必然报错的入口。
pub(super) fn build_local_blank_context_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    tab_id: TabId,
) -> PopupMenu {
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.go_up").to_string())
            .icon(IconName::ArrowUp)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_local_go_up(tab_id, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.refresh").to_string())
            .icon(IconName::RefreshCw)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_local_refresh(tab_id, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.new_local_folder").to_string())
            .icon(IconName::FolderPlus)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_mkdir_modal(tab_id, false, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.new_local_file").to_string())
            .icon(IconName::FilePlus)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_local_new_file_modal(tab_id, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.paste_here").to_string())
            .icon(IconName::Clipboard)
            .on_click(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.sftp_paste_clipboard(tab_id, window, cx);
                });
            }),
    )
}
