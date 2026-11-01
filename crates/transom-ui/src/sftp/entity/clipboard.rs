//! 实体侧的跨列剪贴板：复制、粘贴与跨远端/本地传输编排。
//!
//! 剪贴板与选中集都是面板状态；粘贴产生的传输任务直接进入实体队列，
//! 不再经外壳借还实体。跨域副作用只有队列调度所需的命令投递。

use super::*;
use crate::sftp::side_remote::{
    SftpClipboardData, SftpClipboardEntry, SftpClipboardSource, SftpRemoteCopyParams,
};
use crate::sftp::{SftpActivePane, SftpRemotePaneRef, SftpTransferRequest};
use std::path::PathBuf;
use transom_core::TransferDirection;

impl SftpPanelView {
    /// 触发剪贴板复制（Ctrl+C 或右键复制）。
    pub(crate) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.state.active_pane == SftpActivePane::Local {
            let entries: Vec<SftpClipboardEntry> = if !self.state.local.selection.is_empty() {
                self.state
                    .local
                    .selection
                    .iter()
                    .map(|path| SftpClipboardEntry {
                        is_dir: self
                            .state
                            .local
                            .entries
                            .iter()
                            .find(|entry| &entry.path == path)
                            .is_some_and(|entry| entry.is_dir),
                        path: path.to_string_lossy().to_string(),
                    })
                    .collect()
            } else if let Some(selected) = &self.state.local.selected {
                vec![SftpClipboardEntry {
                    is_dir: self
                        .state
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
            if entries.is_empty() {
                return;
            }
            let count = entries.len();
            self.state.clipboard = Some(SftpClipboardData {
                source: SftpClipboardSource::Local,
                entries,
            });
            self.state
                .set_status(rust_i18n::t!("sftp.sidebar.copied_local", count = count).to_string());
            cx.notify();
            return;
        }

        let target = match self.state.active_pane {
            SftpActivePane::Remote => SftpRemotePaneRef::Main,
            SftpActivePane::SideRemote(connection_id) => SftpRemotePaneRef::Side(connection_id),
            SftpActivePane::Local => return,
        };
        let Some(main_connection) = self.state.main_connection else {
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            return;
        };
        let Some(pane) = self.state.remote_pane(target) else {
            return;
        };
        let paths: Vec<String> = if !pane.selection.is_empty() {
            pane.selection.iter().cloned().collect()
        } else if let Some(selected) = &pane.selected {
            vec![selected.clone()]
        } else {
            Vec::new()
        };
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let entries = paths
            .into_iter()
            .map(|path| SftpClipboardEntry {
                is_dir: pane
                    .entries
                    .iter()
                    .find(|entry| entry.path == path)
                    .is_some_and(|entry| entry.is_dir),
                path,
            })
            .collect();
        self.state.clipboard = Some(SftpClipboardData {
            source: SftpClipboardSource::Remote(connection_id),
            entries,
        });
        let status = if target == SftpRemotePaneRef::Main {
            rust_i18n::t!("sftp.sidebar.copied_remote", count = count)
        } else {
            rust_i18n::t!("sftp.sidebar.copied_side_remote", count = count)
        };
        self.state.set_status(status.to_string());
        cx.notify();
    }

    /// 触发剪贴板粘贴（Ctrl+V 或右键粘贴）。
    pub(crate) fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(clip) = self.state.clipboard.clone() else {
            return;
        };
        let active_pane = self.state.active_pane;
        if active_pane == SftpActivePane::Local {
            let SftpClipboardSource::Remote(source_conn) = clip.source else {
                return;
            };
            let local_dir = self.state.local.path.clone();
            for entry in clip.entries {
                let file_name = entry
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.path)
                    .to_owned();
                let target_path = match transom_core::safe_local_join(&local_dir, &file_name) {
                    Ok(path) => path,
                    Err(_) => {
                        self.state
                            .set_local_error(rust_i18n::t!("sftp.invalid_name").to_string());
                        continue;
                    }
                };
                self.enqueue_transfer(
                    SftpTransferRequest {
                        tab_id: self.tab_id,
                        connection_id: source_conn,
                        direction: TransferDirection::Download,
                        is_dir: entry.is_dir,
                        local_path: target_path,
                        remote_path: entry.path,
                    },
                    cx,
                );
            }
            cx.notify();
            return;
        }

        let target = match active_pane {
            SftpActivePane::Remote => SftpRemotePaneRef::Main,
            SftpActivePane::SideRemote(connection_id) => SftpRemotePaneRef::Side(connection_id),
            SftpActivePane::Local => return,
        };
        let Some(main_connection) = self.state.main_connection else {
            return;
        };
        let Some(target_conn) = self.state.remote_connection(target, Some(main_connection)) else {
            return;
        };
        let Some(target_dir) = self.state.remote_pane(target).map(|pane| pane.path.clone()) else {
            return;
        };

        match clip.source {
            SftpClipboardSource::Local => {
                for entry in clip.entries {
                    let path = PathBuf::from(&entry.path);
                    let file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "upload".into());
                    let target_remote =
                        format!("{}/{}", target_dir.trim_end_matches('/'), file_name);
                    self.enqueue_transfer(
                        SftpTransferRequest {
                            tab_id: self.tab_id,
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
                    for entry in clip.entries {
                        let file_name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
                        let target_remote =
                            format!("{}/{}", target_dir.trim_end_matches('/'), file_name);
                        self.enqueue_remote_transfer(
                            self.tab_id,
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
        cx.notify();
    }
}
