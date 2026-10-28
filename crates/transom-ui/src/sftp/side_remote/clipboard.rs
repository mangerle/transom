//! SFTP 跨列剪贴板与数据中转（Ctrl+C / Ctrl+V 与流式互传）。
//!
//! 设计原理（Why）：
//! - 实现初衷：解耦复杂的跨栏剪贴板复制粘贴动作流，支持本地与多远端间的多向互通；
//! - 核心优势：集中处理本地复制、主远端复制与并列远端复制。

use super::types::{
    SftpActivePane, SftpClipboardData, SftpClipboardEntry, SftpClipboardSource,
    SftpRemoteCopyParams,
};
use crate::app::AppShell;
use crate::sftp::SftpTransferRequest;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{TabId, TransferDirection};

impl AppShell {
    /// 触发剪贴板复制（Ctrl+C 或右键复制）。
    pub(crate) fn sftp_copy_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let main_conn_id = tab.connection_id;
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        let active_pane = panel.active_pane;
        match active_pane {
            SftpActivePane::Local => {
                let entries: Vec<SftpClipboardEntry> = if !panel.local.selection.is_empty() {
                    panel
                        .local
                        .selection
                        .iter()
                        .map(|path| SftpClipboardEntry {
                            is_dir: panel
                                .local
                                .entries
                                .iter()
                                .find(|entry| &entry.path == path)
                                .is_some_and(|entry| entry.is_dir),
                            path: path.to_string_lossy().to_string(),
                        })
                        .collect()
                } else if let Some(selected) = &panel.local.selected {
                    vec![SftpClipboardEntry {
                        is_dir: panel
                            .local
                            .entries
                            .iter()
                            .find(|entry| &entry.path == selected)
                            .is_some_and(|entry| entry.is_dir),
                        path: selected.to_string_lossy().to_string(),
                    }]
                } else {
                    Vec::new()
                };
                if !entries.is_empty() {
                    let count = entries.len();
                    panel.clipboard = Some(SftpClipboardData {
                        source: SftpClipboardSource::Local,
                        entries,
                    });
                    panel.set_status(format!("已复制本地 {count} 个条目，可粘贴至远端"));
                    cx.notify();
                }
            }
            SftpActivePane::Remote => {
                let Some(conn_id) = main_conn_id else {
                    return;
                };
                let paths: Vec<String> = if !panel.remote.selection.is_empty() {
                    panel.remote.selection.iter().cloned().collect()
                } else if let Some(sel) = &panel.remote.selected {
                    vec![sel.clone()]
                } else {
                    Vec::new()
                };
                if !paths.is_empty() {
                    let count = paths.len();
                    let entries = paths
                        .into_iter()
                        .map(|path| SftpClipboardEntry {
                            is_dir: panel
                                .remote
                                .entries
                                .iter()
                                .find(|entry| entry.path == path)
                                .is_some_and(|entry| entry.is_dir),
                            path,
                        })
                        .collect();
                    panel.clipboard = Some(SftpClipboardData {
                        source: SftpClipboardSource::Remote(conn_id),
                        entries,
                    });
                    panel.set_status(format!(
                        "已复制主远端 {count} 个条目，可粘贴至本地或其他远端"
                    ));
                    cx.notify();
                }
            }
            SftpActivePane::SideRemote(conn_id) => {
                if let Some(side) = panel
                    .side_remotes
                    .iter()
                    .find(|side| side.connection_id == conn_id)
                {
                    let paths: Vec<String> = if !side.selection.is_empty() {
                        side.selection.iter().cloned().collect()
                    } else if let Some(sel) = &side.selected {
                        vec![sel.clone()]
                    } else {
                        Vec::new()
                    };
                    if !paths.is_empty() {
                        let count = paths.len();
                        let entries = paths
                            .into_iter()
                            .map(|path| SftpClipboardEntry {
                                is_dir: side
                                    .entries
                                    .iter()
                                    .find(|entry| entry.path == path)
                                    .is_some_and(|entry| entry.is_dir),
                                path,
                            })
                            .collect();
                        panel.clipboard = Some(SftpClipboardData {
                            source: SftpClipboardSource::Remote(conn_id),
                            entries,
                        });
                        panel.set_status(format!(
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
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp() else {
            return;
        };
        let Some(clip) = panel.clipboard.clone() else {
            return;
        };
        let active_pane = panel.active_pane;
        let tab_conn_id = tab.connection_id;

        match active_pane {
            SftpActivePane::Local => {
                if let SftpClipboardSource::Remote(source_conn) = clip.source {
                    // 从远端下载至本地当前目录
                    let local_dir = panel.local.path.clone();
                    for entry in clip.entries {
                        let file_name = entry
                            .path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&entry.path)
                            .to_owned();
                        let target_path =
                            match transom_core::safe_local_join(&local_dir, &file_name) {
                                Ok(path) => path,
                                Err(_) => {
                                    if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
                                        panel.set_error(
                                            rust_i18n::t!("sftp.invalid_name").to_string(),
                                        );
                                    }
                                    continue;
                                }
                            };
                        self.sftp_enqueue_transfer(
                            SftpTransferRequest {
                                tab_id,
                                connection_id: source_conn,
                                direction: TransferDirection::Download,
                                is_dir: entry.is_dir,
                                local_path: target_path,
                                remote_path: entry.path,
                            },
                            cx,
                        );
                    }
                }
            }
            SftpActivePane::Remote => {
                let Some(target_conn) = tab_conn_id else {
                    return;
                };
                let remote_dir = panel.remote.path.clone();
                match clip.source {
                    SftpClipboardSource::Local => {
                        // 从本地上传至主远端
                        for entry in clip.entries {
                            let path = PathBuf::from(&entry.path);
                            let file_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "upload".into());
                            let target_remote =
                                format!("{}/{}", remote_dir.trim_end_matches('/'), file_name);
                            self.sftp_enqueue_transfer(
                                SftpTransferRequest {
                                    tab_id,
                                    connection_id: target_conn,
                                    direction: TransferDirection::Upload,
                                    is_dir: entry.is_dir,
                                    local_path: path,
                                    remote_path: target_remote,
                                },
                                cx,
                            );
                        }
                    }
                    SftpClipboardSource::Remote(src_conn) => {
                        if src_conn != target_conn {
                            // 跨远端互传
                            for entry in clip.entries {
                                let file_name =
                                    entry.path.rsplit('/').next().unwrap_or(&entry.path);
                                let target_remote =
                                    format!("{}/{}", remote_dir.trim_end_matches('/'), file_name);
                                self.sftp_enqueue_remote_transfer(
                                    tab_id,
                                    SftpRemoteCopyParams {
                                        source_connection_id: src_conn,
                                        source_path: entry.path,
                                        target_connection_id: target_conn,
                                        target_path: target_remote,
                                        is_dir: entry.is_dir,
                                    },
                                    cx,
                                );
                            }
                        }
                    }
                }
            }
            SftpActivePane::SideRemote(active_conn) => {
                let Some(side) = panel
                    .side_remotes
                    .iter()
                    .find(|side| side.connection_id == active_conn)
                else {
                    return;
                };
                let target_conn = side.connection_id;
                let target_dir = side.path.clone();

                match clip.source {
                    SftpClipboardSource::Local => {
                        // 从本地上传至并列远端
                        for entry in clip.entries {
                            let path = PathBuf::from(&entry.path);
                            let file_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "upload".into());
                            let target_remote =
                                format!("{}/{}", target_dir.trim_end_matches('/'), file_name);
                            self.sftp_enqueue_transfer(
                                SftpTransferRequest {
                                    tab_id,
                                    connection_id: target_conn,
                                    direction: TransferDirection::Upload,
                                    is_dir: entry.is_dir,
                                    local_path: path,
                                    remote_path: target_remote,
                                },
                                cx,
                            );
                        }
                    }
                    SftpClipboardSource::Remote(src_conn) => {
                        if src_conn != target_conn {
                            // 跨远端互传
                            for entry in clip.entries {
                                let file_name =
                                    entry.path.rsplit('/').next().unwrap_or(&entry.path);
                                let target_remote =
                                    format!("{}/{}", target_dir.trim_end_matches('/'), file_name);
                                self.sftp_enqueue_remote_transfer(
                                    tab_id,
                                    SftpRemoteCopyParams {
                                        source_connection_id: src_conn,
                                        source_path: entry.path,
                                        target_connection_id: target_conn,
                                        target_path: target_remote,
                                        is_dir: entry.is_dir,
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
