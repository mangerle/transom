//! SFTP 远程文件项右键上下文菜单构建。

use crate::app::AppShell;
use crate::sftp::parent_remote;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{SftpOperation, TabId};

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

/// 远端右键上下文菜单构建所需的参数上下文对象。
pub(crate) struct RemoteContextMenuProps {
    pub(crate) app: Entity<AppShell>,
    pub(crate) tab_id: TabId,
    pub(crate) entry_path: String,
    pub(crate) is_dir: bool,
    pub(crate) is_parent_entry: bool,
}

/// 构建远程文件/目录右键菜单。
pub(crate) fn build_remote_context_menu(
    menu: PopupMenu,
    props: RemoteContextMenuProps,
) -> PopupMenu {
    let RemoteContextMenuProps {
        app,
        tab_id,
        entry_path,
        is_dir,
        is_parent_entry,
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
                                this.sftp_go_up(tab_id, window, cx);
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
                                this.sftp_refresh(tab_id, cx);
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

    let mut m = menu.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.download").to_string())
            .icon(IconName::Download)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
                            if panel.remote.selection.contains(&p_dl) {
                                panel.set_remote_primary(p_dl.clone());
                            } else {
                                panel.select_remote(p_dl.clone());
                            }
                        }
                        this.sftp_queue_download(tab_id, cx);
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
                            this.sftp_edit_remote_file(tab_id, p_edit.clone(), window, cx);
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
                                let dest_dir = parent_remote(&p_extract);
                                let op = SftpOperation::Extract {
                                    archive_path: p_extract.clone(),
                                    dest_dir,
                                };
                                this.send_sftp_op(tab_id, op, cx);
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
                                this.sftp_open_extract_modal(
                                    tab_id,
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
                            if let Some(panel) = this.workspace.tab(tab_id).and_then(|t| t.sftp())
                                && panel.remote.selection.contains(&p_archive_gz)
                            {
                                sources = panel.remote.selection.iter().cloned().collect();
                            }
                            if sources.is_empty() {
                                sources.push(p_archive_gz.clone());
                            }
                            let default_name = if sources.len() == 1 {
                                let item_name = sources[0]
                                    .trim_end_matches('/')
                                    .rsplit_once('/')
                                    .map(|(_, n)| n)
                                    .unwrap_or(&sources[0]);
                                format!("{item_name}.tar.gz")
                            } else {
                                "archive.tar.gz".to_string()
                            };
                            this.sftp_open_archive_modal(tab_id, sources, default_name, window, cx);
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
                            if let Some(panel) = this.workspace.tab(tab_id).and_then(|t| t.sftp())
                                && panel.remote.selection.contains(&p_archive_zip)
                            {
                                sources = panel.remote.selection.iter().cloned().collect();
                            }
                            if sources.is_empty() {
                                sources.push(p_archive_zip.clone());
                            }
                            let default_name = if sources.len() == 1 {
                                let item_name = sources[0]
                                    .trim_end_matches('/')
                                    .rsplit_once('/')
                                    .map(|(_, n)| n)
                                    .unwrap_or(&sources[0]);
                                format!("{item_name}.zip")
                            } else {
                                "archive.zip".to_string()
                            };
                            this.sftp_open_archive_modal(tab_id, sources, default_name, window, cx);
                        });
                    }
                }),
        );

    m.item(
        PopupMenuItem::new(rust_i18n::t!("sftp.menu.copy_abs_path").to_string())
            .icon(IconName::Copy)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(p_cp.clone()));
                    app.update(cx, |this, cx| {
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
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
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
                            panel.select_remote(p_sym.clone());
                        }
                        this.sftp_open_symlink_modal(tab_id, true, window, cx);
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
                        this.sftp_request_remote_properties(tab_id, p_prop.clone(), cx);
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
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
                            if panel.remote.selection.contains(&p_exec) {
                                panel.set_remote_primary(p_exec.clone());
                            } else {
                                panel.select_remote(p_exec.clone());
                            }
                        }
                        this.sftp_grant_remote_exec(tab_id, cx);
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
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
                            panel.select_remote(p_chmod.clone());
                        }
                        this.sftp_open_chmod_modal(tab_id, window, cx);
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
                        this.sftp_open_mkdir_modal(tab_id, true, window, cx);
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
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                        {
                            panel.select_remote(p_ren.clone());
                        }
                        this.sftp_open_rename_modal(tab_id, true, window, cx);
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
                        if let Some(panel) =
                            this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            && !panel.remote.selection.contains(&p_del)
                        {
                            panel.select_remote(p_del.clone());
                        }
                        this.sftp_delete_selection(tab_id, cx);
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
                        this.sftp_refresh(tab_id, cx);
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
                        this.sftp_paste_clipboard(tab_id, window, cx);
                    });
                }
            }),
    )
}

/// 远程列空白区域（未命中任何条目）的目录级右键菜单。
///
/// 设计原理（Why）：
/// - 实现初衷：菜单此前只挂在 28px 高的行元素上，目录不满一屏时点下方空白毫无响应，
///   而"返回上一级、刷新、新建、粘贴"本来就是目录级动作，与选中的条目无关；
/// - 核心优势：gpui 的 `context_menu` 在自身 bounds 上命中，且子元素优先响应并停止冒泡，
///   所以容器这一层只在空白处生效，条目上的行菜单不会被它抢走；
/// - 代价与局限：这里刻意只收目录级动作。条目动作（下载、重命名、Chmod 等）仍归行菜单，
///   避免同一功能在两个位置给出不同的可用集合。
pub(crate) fn build_remote_blank_context_menu(
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
                        this.sftp_go_up(tab_id, window, cx);
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
                        this.sftp_refresh(tab_id, cx);
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
                        this.sftp_open_mkdir_modal(tab_id, true, window, cx);
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
