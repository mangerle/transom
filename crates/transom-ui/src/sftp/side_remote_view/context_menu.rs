//! SFTP 并列远端文件列表项右键上下文菜单。
//!
//! 设计原理（Why）：
//! - 实现初衷：将文件列表项复杂的右键上下文菜单项（下载、跨端流式复制、路径复制等）提取为独立函数，
//!   降低主虚拟列表循环与行渲染逻辑的嵌套深度；
//! - 核心优势：集中维护菜单交互行为，各菜单项动作明确。

use crate::app::AppShell;
use crate::sftp::side_remote::{SftpClipboardData, SftpClipboardSource, SftpRemoteCopyParams};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{ConnectionId, TabId};

/// 并列远端右键菜单配置参数。
pub(super) struct SideRemoteMenuContext {
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) entry_path: String,
    pub(super) entry_name: String,
    pub(super) is_dir: bool,
    pub(super) is_parent_entry: bool,
    pub(super) current_dir: String,
}

/// 构建并列远端列表项的右键菜单。
pub(super) fn build_side_remote_context_menu(
    menu: PopupMenu,
    app_entity: Entity<AppShell>,
    ctx: SideRemoteMenuContext,
) -> PopupMenu {
    let tab_id = ctx.tab_id;
    let connection_id = ctx.connection_id;
    let entry_path = ctx.entry_path;
    let entry_name = ctx.entry_name;
    let is_dir = ctx.is_dir;
    let is_parent_entry = ctx.is_parent_entry;
    let current_dir = ctx.current_dir;

    if is_parent_entry {
        let path_up = entry_path;
        let cur_dir = current_dir;
        return menu
            .item(
                PopupMenuItem::new("返回上一级")
                    .icon(IconName::ArrowUp)
                    .on_click({
                        let app = app_entity.clone();
                        let path_up = path_up.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.request_sftp_list_for_connection(
                                    tab_id,
                                    connection_id,
                                    path_up.clone(),
                                    cx,
                                );
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new("刷新")
                    .icon(IconName::RefreshCw)
                    .on_click({
                        let app = app_entity.clone();
                        let cur_dir = cur_dir.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.request_sftp_list_for_connection(
                                    tab_id,
                                    connection_id,
                                    cur_dir.clone(),
                                    cx,
                                );
                            });
                        }
                    }),
            );
    }

    let path_cp = entry_path.clone();
    let path_tr = entry_path.clone();
    let name_tr = entry_name;

    menu.item(
        PopupMenuItem::new("下载到本地")
            .icon(IconName::Download)
            .on_click({
                let app = app_entity.clone();
                let path_tr = path_tr.clone();
                let name_tr = name_tr.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                            let local_target =
                                match transom_core::safe_local_join(&panel.local_path, &name_tr) {
                                    Ok(path) => path,
                                    Err(_) => {
                                        panel.error =
                                            Some(rust_i18n::t!("sftp.invalid_name").to_string());
                                        cx.notify();
                                        return;
                                    }
                                };
                            this.sftp_enqueue_download_path(
                                tab_id,
                                path_tr.clone(),
                                local_target,
                                cx,
                            );
                        }
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new("流式复制到主远端")
            .icon(IconName::ArrowLeftRight)
            .on_click({
                let app = app_entity.clone();
                let path_tr = path_tr.clone();
                let name_tr = name_tr.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        let main_conn = this
                            .workspace
                            .tabs
                            .iter()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| t.connection_id);
                        let main_path = this.sftp_panel_mut_tab(tab_id).map(|p| p.path.clone());
                        if let (Some(target_conn), Some(target_dir)) = (main_conn, main_path) {
                            let target_file =
                                format!("{}/{}", target_dir.trim_end_matches('/'), name_tr);
                            this.sftp_enqueue_remote_transfer(
                                tab_id,
                                SftpRemoteCopyParams {
                                    source_connection_id: connection_id,
                                    source_path: path_tr.clone(),
                                    target_connection_id: target_conn,
                                    target_path: target_file,
                                    is_dir,
                                },
                                cx,
                            );
                        }
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new("复制条目 (Ctrl+C)")
            .icon(IconName::Copy)
            .on_click({
                let app = app_entity.clone();
                let path_tr = path_tr.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                            panel.clipboard = Some(SftpClipboardData {
                                source: SftpClipboardSource::Remote(connection_id),
                                paths: vec![path_tr.clone()],
                            });
                            panel.status = Some(format!("已复制条目: {path_tr}"));
                            cx.notify();
                        }
                    });
                }
            }),
    )
    .item(
        PopupMenuItem::new("复制绝对路径")
            .icon(IconName::Copy)
            .on_click({
                let app = app_entity.clone();
                move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(path_cp.clone()));
                    app.update(cx, |this, cx| {
                        if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                            panel.status = Some(format!("已复制远端路径: {path_cp}"));
                            cx.notify();
                        }
                    });
                }
            }),
    )
}
