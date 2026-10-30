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

use crate::app::AppShell;
use crate::sftp::SftpRemotePaneRef;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::TabId;

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
    pub(crate) app: Entity<AppShell>,
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
            .item(
                PopupMenuItem::new(rust_i18n::t!("sftp.menu.go_up").to_string())
                    .icon(IconName::ArrowUp)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.sftp_go_up_pane(tab_id, target, window, cx);
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
                                this.sftp_refresh_pane(tab_id, target, cx);
                            });
                        }
                    }),
            );
    }

    let p_dl = entry_path.clone();
    let p_edit = entry_path.clone();
    let p_cp = entry_path.clone();
    let p_sym = entry_path.clone();
    let p_prop = entry_path.clone();
    let p_exec = entry_path.clone();
    let p_chmod = entry_path.clone();
    let p_ren = entry_path.clone();
    let p_del = entry_path.clone();
    let p_side_copy = entry_path.clone();

    let mut m = menu.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.download").to_string())
            .icon(IconName::Download)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.select_remote_entry_for_menu(tab_id, target, p_dl.clone(), cx);
                        this.sftp_queue_download_pane(tab_id, target, cx);
                    });
                }
            }),
    );

    if !is_dir {
        m = m.item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.builtin_edit").to_string())
                .icon(IconName::FilePen)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.select_remote_entry_for_menu(tab_id, target, p_edit.clone(), cx);
                            this.sftp_edit_remote_file_pane(
                                tab_id,
                                target,
                                p_edit.clone(),
                                window,
                                cx,
                            );
                        });
                    }
                }),
        );
    }

    if !is_dir && is_archive_name(&entry_path) {
        let p_extract = entry_path.clone();
        let p_extract_new = entry_path.clone();
        m = m
            .item(
                PopupMenuItem::new(rust_i18n::t!("sftp.menu.extract_here").to_string())
                    .icon(IconName::Archive)
                    .on_click({
                        let app = app.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.sftp_extract_here_pane(tab_id, target, p_extract.clone(), cx);
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("sftp.menu.extract_to_new_folder").to_string())
                    .icon(IconName::FolderArchive)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                let file_name = p_extract_new
                                    .rsplit_once('/')
                                    .map(|(_, name)| name)
                                    .unwrap_or(&p_extract_new);
                                let default_dir = strip_archive_suffix(file_name);
                                this.sftp_open_extract_modal_pane(
                                    tab_id,
                                    target,
                                    p_extract_new.clone(),
                                    default_dir,
                                    window,
                                    cx,
                                );
                            });
                        }
                    }),
            );
    }

    let p_archive_gz = entry_path.clone();
    let p_archive_zip = entry_path.clone();
    m = m
        .item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.compress_tar_gz").to_string())
                .icon(IconName::Archive)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            let mut sources = Vec::new();
                            if let Some(panel) =
                                this.workspace.tab(tab_id).and_then(|tab| tab.sftp())
                                && let Some(pane) = panel.remote_pane(target)
                                && pane.selection.contains(&p_archive_gz)
                            {
                                sources = pane.selection.iter().cloned().collect();
                            }
                            if sources.is_empty() {
                                sources.push(p_archive_gz.clone());
                            }
                            let default_name = archive_default_name(&sources, "tar.gz");
                            this.sftp_open_archive_modal_pane(
                                tab_id,
                                target,
                                sources,
                                default_name,
                                window,
                                cx,
                            );
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.compress_zip").to_string())
                .icon(IconName::Archive)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            let mut sources = Vec::new();
                            if let Some(panel) =
                                this.workspace.tab(tab_id).and_then(|tab| tab.sftp())
                                && let Some(pane) = panel.remote_pane(target)
                                && pane.selection.contains(&p_archive_zip)
                            {
                                sources = pane.selection.iter().cloned().collect();
                            }
                            if sources.is_empty() {
                                sources.push(p_archive_zip.clone());
                            }
                            let default_name = archive_default_name(&sources, "zip");
                            this.sftp_open_archive_modal_pane(
                                tab_id,
                                target,
                                sources,
                                default_name,
                                window,
                                cx,
                            );
                        });
                    }
                }),
        );

    if target != SftpRemotePaneRef::Main && has_main_remote {
        let p_to_main = p_side_copy;
        m = m.item(
            PopupMenuItem::new(rust_i18n::t!("sftp.menu.copy_to_main_remote").to_string())
                .icon(IconName::ArrowLeftRight)
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.sftp_copy_remote_entry_to_main(
                                tab_id,
                                target,
                                p_to_main.clone(),
                                is_dir,
                                cx,
                            );
                        });
                    }
                }),
        );
    }

    m.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.copy_abs_path").to_string())
            .icon(IconName::Copy)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(p_cp.clone()));
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                            panel.set_status(
                                rust_i18n::t!("sftp.menu.remote_path_copied", path = p_cp)
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
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.select_remote_entry_for_menu(tab_id, target, p_sym.clone(), cx);
                        this.sftp_open_symlink_modal_pane(tab_id, target, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.properties").to_string())
            .icon(IconName::Info)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_request_remote_properties_pane(
                            tab_id,
                            target,
                            p_prop.clone(),
                            cx,
                        );
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.grant_exec").to_string())
            .icon(IconName::Terminal)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.select_remote_entry_for_menu(tab_id, target, p_exec.clone(), cx);
                        this.sftp_grant_remote_exec_pane(tab_id, target, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.chmod").to_string())
            .icon(IconName::Settings)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.select_remote_entry_for_menu(tab_id, target, p_chmod.clone(), cx);
                        this.sftp_open_chmod_modal_for(tab_id, target, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.new_remote_folder").to_string())
            .icon(IconName::FolderPlus)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_mkdir_modal_for(tab_id, target, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.rename").to_string())
            .icon(IconName::Pencil)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.select_remote_entry_for_menu(tab_id, target, p_ren.clone(), cx);
                        this.sftp_open_rename_modal_for(tab_id, target, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.delete").to_string())
            .icon(IconName::Trash)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.select_remote_entry_for_menu(tab_id, target, p_del.clone(), cx);
                        this.sftp_delete_selection_for(tab_id, target, cx);
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
                        this.sftp_refresh_pane(tab_id, target, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.copy_entries").to_string())
            .icon(IconName::Copy)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.activate_remote_pane(tab_id, target, cx);
                        this.sftp_copy_selection(tab_id, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.paste_here").to_string())
            .icon(IconName::Clipboard)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.activate_remote_pane(tab_id, target, cx);
                        this.sftp_paste_clipboard(tab_id, window, cx);
                    });
                }
            }),
    )
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
    app: Entity<AppShell>,
    tab_id: TabId,
    target: SftpRemotePaneRef,
) -> PopupMenu {
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.go_up").to_string())
            .icon(IconName::ArrowUp)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_go_up_pane(tab_id, target, window, cx);
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
                        this.sftp_refresh_pane(tab_id, target, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.new_remote_folder").to_string())
            .icon(IconName::FolderPlus)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.sftp_open_mkdir_modal_for(tab_id, target, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.paste_here").to_string())
            .icon(IconName::Clipboard)
            .on_click(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.activate_remote_pane(tab_id, target, cx);
                    this.sftp_paste_clipboard(tab_id, window, cx);
                });
            }),
    )
}
