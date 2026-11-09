//! 日志、统计、文件浏览与传输进度。
//!
//! 设计原理（Why）：事件处理只按请求 id 定位标签页，具体状态流转收敛到
//! [`ContainerWorkspace`] 的具名方法（见 `apply.rs`）。

use super::super::*;

impl AppShell {
    pub(super) fn handle_file_stats_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerAppEvent::ExecReady {
                tab_id,
                request_id,
                io,
                exit,
            } => self.on_container_exec_ready(tab_id, request_id, *io, exit, window, cx),
            ContainerAppEvent::LogsChunk { request_id, bytes } => {
                let Some(tab_id) = self.state.container_request_tab(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_logs_chunk(request_id, &bytes)
                    });
                }
            }
            ContainerAppEvent::LogsClosed { request_id } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| workspace.apply_logs_closed(request_id));
                }
            }
            ContainerAppEvent::StatsUpdated {
                request_id,
                container: _,
                stats,
            } => {
                let Some(tab_id) = self.state.container_request_tab(request_id, cx) else {
                    return;
                };
                log::debug!("收到容器统计更新，请求: {request_id}");
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_stats_updated(request_id, stats)
                    });
                }
            }
            ContainerAppEvent::StatsUnavailable { request_id, error } => {
                let Some(tab_id) = self.state.container_request_tab(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_stats_unavailable(request_id, &error)
                    });
                }
            }
            ContainerAppEvent::DirListed {
                request_id,
                path,
                entries,
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_dir_listed(request_id, path, entries)
                    });
                }
            }
            ContainerAppEvent::FilePreview {
                request_id,
                path,
                content,
                truncated,
                binary,
                ..
            } => {
                let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_file_preview(path, content, truncated, binary)
                    });
                }
            }
            ContainerAppEvent::TransferProgress {
                request_id,
                transferred,
                total,
            } => {
                let Some(tab_id) = self.state.container_request_tab(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_transfer_progress(transferred, total)
                    });
                }
            }
            ContainerAppEvent::BuildProgress { request_id, line } => {
                let Some(tab_id) = self.state.container_request_tab(request_id, cx) else {
                    return;
                };
                if let Some(workspace) = self.state.container_workspace(tab_id) {
                    workspace.update(cx, |workspace, _| {
                        workspace.apply_build_progress(request_id, &line)
                    });
                }
            }
            _ => {}
        }
    }
}
