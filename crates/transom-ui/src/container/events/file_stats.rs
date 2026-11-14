//! 日志、统计、文件浏览与传输进度。
//!
//! 设计原理（Why）：事件处理只按请求 id 定位标签页，具体状态流转收敛到
//! [`ContainerWorkspace`] 的具名方法（见 `apply.rs`）。

use super::super::*;

fn apply_by_request(
    shell: &mut AppShell,
    request_id: ContainerRequestId,
    consume: bool,
    cx: &mut Context<AppShell>,
    apply: impl FnOnce(&mut ContainerWorkspace),
) {
    let tab_id = if consume {
        shell.state.take_container_request(request_id, cx)
    } else {
        shell.state.container_request_tab(request_id, cx)
    };
    if let Some(tab_id) = tab_id
        && let Some(workspace) = shell.state.container_workspace(tab_id)
    {
        workspace.update(cx, |workspace, _| apply(workspace));
    }
}

pub(super) fn handle_file_stats_event(
    shell: &mut AppShell,
    event: ContainerAppEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        ContainerAppEvent::ExecReady {
            tab_id,
            request_id,
            io,
            exit,
        } => super::terminal::on_container_exec_ready(
            shell, tab_id, request_id, *io, exit, window, cx,
        ),
        ContainerAppEvent::LogsChunk { request_id, bytes } => {
            apply_by_request(shell, request_id, false, cx, |w| {
                w.apply_logs_chunk(request_id, &bytes);
            });
        }
        ContainerAppEvent::LogsClosed { request_id } => {
            apply_by_request(shell, request_id, true, cx, |w| {
                w.apply_logs_closed(request_id);
            });
        }
        ContainerAppEvent::StatsUpdated {
            request_id,
            container: _,
            stats,
        } => {
            log::debug!("收到容器统计更新，请求: {request_id}");
            apply_by_request(shell, request_id, false, cx, |w| {
                w.apply_stats_updated(request_id, stats);
            });
        }
        ContainerAppEvent::StatsUnavailable { request_id, error } => {
            apply_by_request(shell, request_id, false, cx, |w| {
                w.apply_stats_unavailable(request_id, &error);
            });
        }
        ContainerAppEvent::DirListed {
            request_id,
            path,
            entries,
        } => {
            apply_by_request(shell, request_id, true, cx, |w| {
                w.apply_dir_listed(request_id, path, entries);
            });
        }
        ContainerAppEvent::FilePreview {
            request_id,
            path,
            content,
            truncated,
            binary,
            ..
        } => {
            apply_by_request(shell, request_id, true, cx, |w| {
                w.apply_file_preview(path, content, truncated, binary);
            });
        }
        ContainerAppEvent::TransferProgress {
            request_id,
            transferred,
            total,
        } => {
            apply_by_request(shell, request_id, false, cx, |w| {
                w.apply_transfer_progress(transferred, total);
            });
        }
        ContainerAppEvent::BuildProgress { request_id, line } => {
            apply_by_request(shell, request_id, false, cx, |w| {
                w.apply_build_progress(request_id, &line);
            });
        }
        _ => {}
    }
}
