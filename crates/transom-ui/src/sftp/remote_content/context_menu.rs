//! SFTP 远程文件项右键上下文菜单构建。

use crate::app::AppShell;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::TabId;

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
                PopupMenuItem::new("返回上一级")
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
                PopupMenuItem::new("刷新")
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
    let p_chmod = entry_path.clone();
    let p_ren = entry_path.clone();
    let p_del = entry_path.clone();

    let mut m = menu.item(
        PopupMenuItem::new("下载")
            .icon(IconName::Download)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.sftp.as_mut())
                        {
                            if panel.remote_selection.contains(&p_dl) {
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
            PopupMenuItem::new("内置编辑")
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

    m.item(
        PopupMenuItem::new("复制绝对路径")
            .icon(IconName::Copy)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(p_cp.clone()));
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.sftp.as_mut())
                        {
                            panel.status = Some(format!("已复制远端路径: {p_cp}"));
                        }
                        cx.notify();
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new("创建软链接")
            .icon(IconName::Link2)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.sftp.as_mut())
                        {
                            panel.select_remote(p_sym.clone());
                        }
                        this.sftp_open_symlink_modal(tab_id, true, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new("查看属性")
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
        PopupMenuItem::new("修改权限 (Chmod)")
            .icon(IconName::Settings)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.sftp.as_mut())
                        {
                            panel.select_remote(p_chmod.clone());
                        }
                        this.sftp_open_chmod_modal(tab_id, window, cx);
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new("新建远端文件夹")
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
        PopupMenuItem::new("重命名")
            .icon(IconName::Pencil)
            .on_click({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.sftp.as_mut())
                        {
                            panel.select_remote(p_ren.clone());
                        }
                        this.sftp_open_rename_modal(tab_id, true, window, cx);
                    });
                }
            }),
    )
    .item(PopupMenuItem::new("删除").icon(IconName::Trash).on_click({
        let app = app.clone();
        move |_, _, cx| {
            app.update(cx, |this, cx| {
                if let Some(panel) = this
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp.as_mut())
                    && !panel.remote_selection.contains(&p_del)
                {
                    panel.select_remote(p_del.clone());
                }
                this.sftp_delete_selection(tab_id, cx);
            });
        }
    }))
    .item(
        PopupMenuItem::new("刷新")
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
        PopupMenuItem::new("复制条目 (Ctrl+C)")
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
        PopupMenuItem::new("粘贴至此处 (Ctrl+V)")
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
