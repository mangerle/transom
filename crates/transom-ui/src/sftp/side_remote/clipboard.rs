//! SFTP 跨列剪贴板与数据中转（Ctrl+C / Ctrl+V 与流式互传）。
//!
//! 设计原理（Why）：
//! - 实现初衷：解耦复杂的跨栏剪贴板复制粘贴动作流，支持本地与多远端间的多向互通；
//! - 核心优势：集中处理本地复制、主远端复制与并列远端复制。

use super::types::{SftpActivePane, SftpClipboardData, SftpClipboardSource, SftpRemoteCopyParams};
use crate::app::AppShell;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;

impl AppShell {
    /// 触发剪贴板复制（Ctrl+C 或右键复制）。
    pub(crate) fn sftp_copy_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let main_conn_id = tab.connection_id;
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        let active_pane = panel.active_pane;
        match active_pane {
            SftpActivePane::Local => {
                let paths: Vec<String> = if !panel.local_selection.is_empty() {
                    panel
                        .local_selection
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect()
                } else if let Some(sel) = &panel.local_selected {
                    vec![sel.to_string_lossy().to_string()]
                } else {
                    Vec::new()
                };
                if !paths.is_empty() {
                    let count = paths.len();
                    panel.clipboard = Some(SftpClipboardData {
                        source: SftpClipboardSource::Local,
                        paths,
                    });
                    panel.status = Some(format!("已复制本地 {count} 个条目，可粘贴至远端"));
                    cx.notify();
                }
            }
            SftpActivePane::Remote => {
                let Some(conn_id) = main_conn_id else {
                    return;
                };
                let paths: Vec<String> = if !panel.remote_selection.is_empty() {
                    panel.remote_selection.iter().cloned().collect()
                } else if let Some(sel) = &panel.selected {
                    vec![sel.clone()]
                } else {
                    Vec::new()
                };
                if !paths.is_empty() {
                    let count = paths.len();
                    panel.clipboard = Some(SftpClipboardData {
                        source: SftpClipboardSource::Remote(conn_id),
                        paths,
                    });
                    panel.status = Some(format!(
                        "已复制主远端 {count} 个条目，可粘贴至本地或其他远端"
                    ));
                    cx.notify();
                }
            }
            SftpActivePane::SideRemote(idx) => {
                if let Some(side) = panel.side_remotes.get(idx) {
                    let conn_id = side.connection_id;
                    let paths: Vec<String> = if !side.selection.is_empty() {
                        side.selection.iter().cloned().collect()
                    } else if let Some(sel) = &side.selected {
                        vec![sel.clone()]
                    } else {
                        Vec::new()
                    };
                    if !paths.is_empty() {
                        let count = paths.len();
                        panel.clipboard = Some(SftpClipboardData {
                            source: SftpClipboardSource::Remote(conn_id),
                            paths,
                        });
                        panel.status = Some(format!(
                            "已复制并列远端 {count} 个条目，可粘贴至本地或其他远端"
                        ));
                        cx.notify();
                    }
                }
            }
        }
    }

    /// 触发剪贴板粘贴（Ctrl+V 或右键粘贴）。
    pub(crate) fn sftp_paste_clipboard(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_ref() else {
            return;
        };
        let Some(clip) = panel.clipboard.clone() else {
            return;
        };
        let active_pane = panel.active_pane;
        let tab_conn_id = tab.connection_id;

        match active_pane {
            SftpActivePane::Local => {
                match clip.source {
                    SftpClipboardSource::Local => {
                        // 本地到本地暂不触发
                    }
                    SftpClipboardSource::Remote(_conn_id) => {
                        // 从远端下载至本地当前目录
                        let local_dir = panel.local_path.clone();
                        for path in clip.paths {
                            let file_name = path.rsplit('/').next().unwrap_or(&path);
                            let target_path = local_dir.join(file_name);
                            self.sftp_enqueue_download_path(tab_id, path, target_path, cx);
                        }
                    }
                }
            }
            SftpActivePane::Remote => {
                let Some(target_conn) = tab_conn_id else {
                    return;
                };
                let remote_dir = panel.path.clone();
                match clip.source {
                    SftpClipboardSource::Local => {
                        // 从本地上传至主远端
                        for path_str in clip.paths {
                            let path = PathBuf::from(&path_str);
                            let file_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "upload".into());
                            let target_remote =
                                format!("{}/{}", remote_dir.trim_end_matches('/'), file_name);
                            self.sftp_enqueue_upload_path(tab_id, path, target_remote, cx);
                        }
                    }
                    SftpClipboardSource::Remote(src_conn) => {
                        if src_conn != target_conn {
                            // 跨远端互传
                            for path in clip.paths {
                                let file_name = path.rsplit('/').next().unwrap_or(&path);
                                let target_remote =
                                    format!("{}/{}", remote_dir.trim_end_matches('/'), file_name);
                                self.sftp_enqueue_remote_transfer(
                                    tab_id,
                                    SftpRemoteCopyParams {
                                        source_connection_id: src_conn,
                                        source_path: path,
                                        target_connection_id: target_conn,
                                        target_path: target_remote,
                                        is_dir: false,
                                    },
                                    cx,
                                );
                            }
                        }
                    }
                }
            }
            SftpActivePane::SideRemote(idx) => {
                let target_conn = panel.side_remotes.get(idx).map(|s| s.connection_id);
                let target_dir = panel.side_remotes.get(idx).map(|s| s.path.clone());
                let (Some(target_conn), Some(target_dir)) = (target_conn, target_dir) else {
                    return;
                };

                match clip.source {
                    SftpClipboardSource::Local => {
                        // 从本地上传至并列远端
                        for path_str in clip.paths {
                            let path = PathBuf::from(&path_str);
                            let file_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "upload".into());
                            let target_remote =
                                format!("{}/{}", target_dir.trim_end_matches('/'), file_name);
                            self.sftp_enqueue_upload_path(tab_id, path, target_remote, cx);
                        }
                    }
                    SftpClipboardSource::Remote(src_conn) => {
                        if src_conn != target_conn {
                            // 跨远端互传
                            for path in clip.paths {
                                let file_name = path.rsplit('/').next().unwrap_or(&path);
                                let target_remote =
                                    format!("{}/{}", target_dir.trim_end_matches('/'), file_name);
                                self.sftp_enqueue_remote_transfer(
                                    tab_id,
                                    SftpRemoteCopyParams {
                                        source_connection_id: src_conn,
                                        source_path: path,
                                        target_connection_id: target_conn,
                                        target_path: target_remote,
                                        is_dir: false,
                                    },
                                    cx,
                                );
                            }
                        }
                    }
                }
            }
        }
        cx.notify();
    }
}
