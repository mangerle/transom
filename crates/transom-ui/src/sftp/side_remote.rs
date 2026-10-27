//! SFTP 并列多远端挂载、折叠状态、跨远端剪贴板与拖放交互模块。
//!
//! 设计原理（Why）：
//! - 实现初衷：支持在单个终端标签的 SFTP 视图中挂载多个远端会话（本地 + 远端 A + 远端 B...），
//!   免去在多个标签页之间反复来回切换，直观实现同屏比对与跨主机流式复制；
//! - 核心优势：
//!   1. 各列（本地列与并列远端列）均支持独立一键收起折叠，最大化屏幕有效阅读空间；
//!   2. 支持跨列拖拽（Drag & Drop）、Ctrl+C / Ctrl+V 剪贴板与右键菜单三重交互闭环；
//!   3. 挂载器支持复用当前已建立连接的 SSH 会话（秒级挂载）或快速连接已保存会话；
//! - 代价与局限：并列多列受屏幕实际宽度影响，单列过窄时由横向自适应和列折叠共同吸收。

use crate::app::AppShell;
use crate::palette::sftp_text_input;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use transom_core::{
    ConnectionId, RemoteFile, TabId, TransferDirection, TransferMode, TransferTask,
};

/// 跨栏剪贴板数据来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SftpClipboardSource {
    /// 来自本地文件系统
    Local,
    /// 来自远端文件系统（携带源连接 ID）
    Remote(ConnectionId),
}

/// 跨栏剪贴板数据结构（供 Ctrl+C / Ctrl+V 使用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpClipboardData {
    pub(crate) source: SftpClipboardSource,
    pub(crate) paths: Vec<String>,
}

/// 跨列文件拖拽载荷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpDragItem {
    pub(crate) source_tab_id: TabId,
    pub(crate) source_connection_id: Option<ConnectionId>,
    pub(crate) paths: Vec<String>,
    pub(crate) file_name: String,
    pub(crate) is_dir: bool,
}

/// 跨列文件拖拽视觉预览浮层。
pub(crate) struct SftpDragPreview {
    pub(crate) name: String,
    pub(crate) count: usize,
}

impl SftpDragPreview {
    pub(crate) fn new(name: String, count: usize) -> Self {
        Self { name, count }
    }
}

impl Render for SftpDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1()
            .gap_1p5()
            .rounded(px(4.0))
            .bg(hsla(0.0, 0.0, 0.15, 0.95))
            .text_color(hsla(0.0, 0.0, 0.95, 1.0))
            .text_xs()
            .items_center()
            .child(Icon::new(IconName::File).xsmall())
            .child(self.name.clone())
            .when(self.count > 1, |this| {
                this.child(
                    div()
                        .px_1()
                        .rounded(px(2.0))
                        .bg(hsla(0.0, 0.0, 0.3, 0.9))
                        .child(format!("+{}", self.count - 1)),
                )
            })
    }
}

/// SFTP 激活的焦点栏位。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SftpActivePane {
    Local,
    #[default]
    Remote,
    SideRemote(usize),
}

/// 跨远端互传任务参数对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpRemoteCopyParams {
    pub(crate) source_connection_id: ConnectionId,
    pub(crate) source_path: String,
    pub(crate) target_connection_id: ConnectionId,
    pub(crate) target_path: String,
    pub(crate) is_dir: bool,
}

/// 并列借调的远端文件管理器状态。
pub(crate) struct SftpSideRemoteState {
    pub(crate) connection_id: ConnectionId,
    pub(crate) title: String,
    pub(crate) path: String,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) entries: Vec<RemoteFile>,
    pub(crate) selected: Option<String>,
    pub(crate) selection: BTreeSet<String>,
    pub(crate) scroll_handle: VirtualListScrollHandle,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) collapsed: bool,
}

impl SftpSideRemoteState {
    pub(crate) fn new(
        connection_id: ConnectionId,
        title: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Self {
        let path_input = sftp_text_input(window, cx, &path, "远端路径...");
        let filter_input = sftp_text_input(window, cx, "", "过滤文件 (模糊匹配)...");
        Self {
            connection_id,
            title,
            path,
            path_input,
            filter_input,
            entries: Vec::new(),
            selected: None,
            selection: BTreeSet::new(),
            scroll_handle: VirtualListScrollHandle::new(),
            loading: true,
            error: None,
            collapsed: false,
        }
    }
}

impl AppShell {
    /// 挂载并列远端会话到当前 SFTP 面板。
    pub(crate) fn sftp_mount_side_remote(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        title: String,
        initial_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        // 避免重复挂载相同连接
        if panel
            .side_remotes
            .iter()
            .any(|s| s.connection_id == connection_id)
        {
            panel.status = Some(format!("该远端会话已挂载: {title}"));
            cx.notify();
            return;
        }

        let side_state =
            SftpSideRemoteState::new(connection_id, title, initial_path.clone(), window, cx);
        panel.side_remotes.push(side_state);
        panel.mount_menu_open = false;
        self.request_sftp_list_for_connection(tab_id, connection_id, initial_path, cx);
        cx.notify();
    }

    /// 关闭并列远端列。
    pub(crate) fn sftp_close_side_remote(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && index < panel.side_remotes.len()
        {
            panel.side_remotes.remove(index);
            if panel.active_pane == SftpActivePane::SideRemote(index) {
                panel.active_pane = SftpActivePane::Remote;
            }
            cx.notify();
        }
    }

    /// 切换并列远端列的展开与折叠。
    pub(crate) fn sftp_toggle_side_remote_collapse(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(side) = panel.side_remotes.get_mut(index)
        {
            side.collapsed = !side.collapsed;
            cx.notify();
        }
    }

    /// 切换本地列的展开与折叠。
    pub(crate) fn sftp_toggle_local_collapse(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.local_collapsed = !panel.local_collapsed;
            cx.notify();
        }
    }

    /// 切换主远端列的展开与折叠。
    pub(crate) fn sftp_toggle_remote_collapse(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.remote_collapsed = !panel.remote_collapsed;
            cx.notify();
        }
    }

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

    /// 跨远端互传任务入队。
    pub(crate) fn sftp_enqueue_remote_transfer(
        &mut self,
        tab_id: TabId,
        params: SftpRemoteCopyParams,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let src_path = params.source_path.clone();
        let tgt_path = params.target_path.clone();
        let task = TransferTask::new_remote_copy(
            params.source_connection_id,
            params.source_path,
            params.target_connection_id,
            params.target_path,
            None,
            params.is_dir,
        );
        panel.queue.push(task);
        panel.status = Some(format!("已添加跨远端传输任务: {src_path} → {tgt_path}"));
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 将单个下载路径加入传输队列。
    pub(crate) fn sftp_enqueue_download_path(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let mode = if panel.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut task = TransferTask::new(
            TransferDirection::Download,
            local_path,
            remote_path.clone(),
            None,
            mode,
        );
        task.verify = verify;
        panel.queue.push(task);
        panel.status = Some(format!("已加入下载: {remote_path}"));
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 将单个上传路径加入传输队列。
    pub(crate) fn sftp_enqueue_upload_path(
        &mut self,
        tab_id: TabId,
        local_path: PathBuf,
        target_remote: String,
        cx: &mut Context<Self>,
    ) {
        let verify = self.settings.sftp.verify_after_transfer;
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let mode = if panel.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut task = TransferTask::new(
            TransferDirection::Upload,
            local_path.clone(),
            target_remote.clone(),
            None,
            mode,
        );
        task.verify = verify;
        panel.queue.push(task);
        panel.status = Some(format!("已加入上传: {}", local_path.display()));
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 辅助方法：直接按 tab_id 获取 sftp 面板可变借用。
    pub(crate) fn sftp_panel_mut_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<&mut super::SftpPanelState> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
    }

    /// 请求特定连接的远端目录列表（用于并列远端）。
    pub(crate) fn request_sftp_list_for_connection(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        path: String,
        _cx: &mut Context<Self>,
    ) {
        use transom_core::{AppCommand, SftpOperation, TransferId};
        let op_id = TransferId::new();
        let cmd = AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::List { path },
        };
        let _ = self.services.command_tx.send(cmd);
    }
}
