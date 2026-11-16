//! 容器系统级全局事件处理：存储占用分析、系统深度联合清理与文件变更差异。

use super::*;
use crate::format::format_size;

/// 全局存储分析加载完成。
pub(super) fn on_container_system_df_loaded(
    app_state: &mut AppState,
    request_id: ContainerRequestId,
    usage: transom_core::ContainerSystemDiskUsage,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = app_state.take_container_request(request_id, cx) else {
        return;
    };
    if let Some(workspace) = app_state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, _| {
            workspace.apply_system_df_loaded(request_id, usage)
        });
    }
}

/// 系统联合深度清理执行完毕。
pub(super) fn on_container_system_pruned(
    app_state: &mut AppState,
    request_id: ContainerRequestId,
    report: transom_core::ContainerPruneReport,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = app_state.take_container_request(request_id, cx) else {
        return;
    };

    if let Some(workspace) = app_state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, _| workspace.apply_system_pruned());
    }

    let reclaimed_str = format_size(report.reclaimed);
    let succeeded = report.errors.is_empty();
    let msg = if succeeded {
        rust_i18n::t!(
            "container.system_df.prune_success",
            reclaimed = reclaimed_str
        )
        .to_string()
    } else {
        rust_i18n::t!(
            "container.system_df.prune_partial",
            count = report.errors.len(),
            reclaimed = reclaimed_str,
            errors = report.errors.join("；")
        )
        .to_string()
    };
    app_state.ui.window_requests.push_toast(msg, succeeded);

    // 清理完成后，重新统计磁盘占用并刷新各资源列表
    if let Some(workspace) = app_state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, cx| workspace.refresh_system_df(cx));
    }
    if let Some(workspace) = app_state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, _| workspace.refresh_all());
    }
}

/// 容器文件系统差异对比完成。
pub(super) fn on_container_diffed(
    app_state: &mut AppState,
    request_id: ContainerRequestId,
    container: String,
    changes: Vec<transom_core::ContainerFileSystemChange>,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = app_state.take_container_request(request_id, cx) else {
        return;
    };
    if let Some(workspace) = app_state.container_workspace(tab_id) {
        workspace.update(cx, |workspace, _| {
            workspace.apply_diffed(request_id, container, changes)
        });
    }
}

/// 容器引擎长连接事件流收到事件。
pub(super) fn on_container_engine_event_received(
    app_state: &mut AppState,
    tab_id: TabId,
    event: transom_core::EngineEvent,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(current_section) = app_state
        .container_workspace(tab_id)
        .map(|workspace| workspace.read(cx).state.section)
    else {
        return;
    };

    // 0. 事件流重连后的全量对账：断线期间漏掉的事件无法逐条补发。
    if event.resource_type == "engine" && event.action == "resync" {
        log::debug!("容器事件流重连，触发工作区全量刷新");
        if let Some(workspace) = app_state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| workspace.refresh_all());
        }
        return;
    }

    // 1. 如果是容器异常退出或 OOM，向用户弹出警告 Toast
    if event.is_abnormal_container_exit() {
        let fallback_id = &event.id[..event.id.len().min(12)];
        let name = event.name.as_deref().unwrap_or(fallback_id);
        let msg = if event.action == "oom" {
            rust_i18n::t!("container.event.oom_killed", name = name).to_string()
        } else {
            let code_val = event.exit_code.unwrap_or(-1);
            rust_i18n::t!(
                "container.event.abnormal_exit",
                name = name,
                code = code_val
            )
            .to_string()
        };
        app_state.ui.window_requests.push_toast(msg, false);
    }

    // 2. 根据事件类型触发当前激活分类的自动刷新
    match event.resource_type.as_str() {
        "container" => {
            if matches!(
                current_section,
                ContainerSection::Containers | ContainerSection::Compose
            ) && let Some(workspace) = app_state.container_workspace(tab_id)
            {
                workspace.update(cx, |workspace, cx| workspace.schedule_refresh(cx));
            }
        }
        "image" => {
            if current_section == ContainerSection::Images
                && let Some(workspace) = app_state.container_workspace(tab_id)
            {
                workspace.update(cx, |workspace, cx| workspace.schedule_refresh(cx));
            }
        }
        "volume" => {
            if current_section == ContainerSection::Volumes
                && let Some(workspace) = app_state.container_workspace(tab_id)
            {
                workspace.update(cx, |workspace, cx| workspace.schedule_refresh(cx));
            }
        }
        "network" if current_section == ContainerSection::Networks => {
            if let Some(workspace) = app_state.container_workspace(tab_id) {
                workspace.update(cx, |workspace, cx| workspace.schedule_refresh(cx));
            }
        }
        _ => {}
    }
}
