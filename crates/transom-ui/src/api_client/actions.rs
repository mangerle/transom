//! API 客户端窗口动作：打开项目标签、导入文件、持久化与事件落地。

use super::{ApiSidebarEvent, ApiWorkspace, ApiWorkspaceEvent, ApiWorkspaceServices};
use crate::app::{AppShell, AppState, WorkspaceTab};
use gpui_kit::*;
use transom_api::{
    ApiExportFormat, ApiImportFormat, ApiProject, ApiProjectId, MAX_IMPORT_BYTES, export_project,
    import_projects,
};
use transom_core::TabId;

impl AppState {
    /// 当前活动标签对应的 API 项目 id。
    pub(crate) fn active_api_project_id(&self) -> Option<ApiProjectId> {
        self.workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
            .and_then(WorkspaceTab::api_project_id)
    }
}

/// 侧栏事件订阅入口：只排队，所有窗口动作统一在渲染期执行。
pub(crate) fn on_sidebar_event(
    state: &mut AppState,
    event: &ApiSidebarEvent,
    cx: &mut Context<AppShell>,
) {
    state.ui.window_requests.push_api_sidebar(event.clone());
    cx.notify();
}

/// API 工作区事件订阅入口：事件自带完整项目快照，外壳不回读派发实体。
pub(crate) fn on_workspace_event(
    state: &mut AppState,
    tab_id: TabId,
    event: &ApiWorkspaceEvent,
    cx: &mut Context<AppShell>,
) {
    state
        .ui
        .window_requests
        .push_api_workspace(tab_id, event.clone());
    cx.notify();
}

/// 打开或聚焦一个 API 项目标签。
pub(crate) fn open_api_project(
    state: &mut AppState,
    project: ApiProject,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(existing) = state.workspace.api_tab_for_project(project.id) {
        state.ui.window_requests.push_activate_tab(existing);
        return;
    }

    let tab_id = TabId::new();
    let title = project.display_name().to_string();
    let services = ApiWorkspaceServices {
        http: state.runtime.services.api_http.clone(),
        websocket: state.runtime.services.api_ws.clone(),
    };
    let workspace = cx.new(|cx| ApiWorkspace::new(project.clone(), services, window, cx));
    let subscription = cx.subscribe(
        &workspace,
        move |shell, _workspace, event: &ApiWorkspaceEvent, cx| {
            super::on_workspace_event(&mut shell.state, tab_id, event, cx);
        },
    );
    state
        .runtime
        .tasks
        .subscriptions
        .api_workspaces
        .insert(tab_id, subscription);
    let observer = cx.observe(&workspace, |_, _, cx| cx.notify());
    state
        .runtime
        .tasks
        .subscriptions
        .api_workspace_observers
        .insert(tab_id, observer);

    let tab = WorkspaceTab::new_api_client(tab_id, title, project.id, workspace);
    state.workspace.create_group_for_tab(&tab);
    state.workspace.tabs.push(tab);
    state.workspace.active_tab = Some(tab_id);
    state.present_active_group();
    log::info!(
        "已打开 API 项目，项目: {}，标签: {}",
        project.display_name(),
        tab_id
    );
    cx.notify();
}

/// 处理侧栏事件队列中的窗口动作。
pub(crate) fn on_sidebar_request(
    state: &mut AppState,
    event: ApiSidebarEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        ApiSidebarEvent::OpenProject(project) => open_api_project(state, project, window, cx),
        ApiSidebarEvent::PersistRegistry {
            registry,
            changed_project,
            removed_project,
            notice,
        } => {
            if let Err(error) = state.runtime.persistence.save_api_registry(&registry) {
                state.set_error(error.detail.unwrap_or_else(|| format!("{:?}", error.code)));
            }
            if let Some(project) = changed_project {
                update_open_project(state, &project, cx);
            }
            if let Some(project_id) = removed_project
                && let Some(tab_id) = state.workspace.api_tab_for_project(project_id)
            {
                state.ui.window_requests.push_close_tab(tab_id);
            }
            if let Some(notice) = notice {
                state.set_transient_status(notice, cx);
            }
            cx.notify();
        }
        ApiSidebarEvent::ImportProject => start_import(state, window, cx),
        ApiSidebarEvent::ExportProject { project, format } => {
            start_export(state, project, format, window, cx)
        }
    }
}

/// 处理 API 工作区的保存事件。
pub(crate) fn on_workspace_request(
    state: &mut AppState,
    tab_id: TabId,
    event: ApiWorkspaceEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        ApiWorkspaceEvent::PersistProject { project, notice } => {
            let sidebar = state.features.api_sidebar.clone();
            let registry = sidebar.update(cx, |view, cx| {
                view.apply_project_snapshot(project.clone(), cx);
                view.state.registry.clone()
            });
            if let Err(error) = state.runtime.persistence.save_api_registry(&registry) {
                state.set_error(error.detail.unwrap_or_else(|| format!("{:?}", error.code)));
            }
            update_open_project(state, &project, cx);
            if let Some(notice) = notice {
                state.set_transient_status(notice, cx);
            }
            if state.workspace.tab(tab_id).is_some() {
                cx.notify();
            }
        }
    }
}

fn update_open_project(state: &mut AppState, project: &ApiProject, cx: &mut Context<AppShell>) {
    for tab in state.workspace.tabs.iter_mut() {
        if tab.api_project_id() != Some(project.id) {
            continue;
        }
        tab.title.clone_from(&project.name);
        if let Some(workspace) = tab.api_workspace() {
            workspace.update(cx, |workspace, cx| {
                workspace.apply_project_snapshot(project.clone(), cx)
            });
        }
    }
}

fn start_import(state: &mut AppState, window: &mut Window, cx: &mut Context<AppShell>) {
    let Some(path) = rfd::FileDialog::new()
        .set_title(rust_i18n::t!("api.project.import_title").to_string())
        .add_filter("API project", &["json", "yaml", "yml", "sh", "curl", "txt"])
        .pick_file()
    else {
        return;
    };

    let task = cx.spawn_in(window, async move |this, cx| {
        let parsed = crate::app::spawn_blocking_local(move || read_import_file(&path)).await;
        let _ = this.update_in(cx, |shell, _window, cx| match parsed {
            Some(Ok(projects)) => {
                let sidebar = shell.state.features.api_sidebar.clone();
                sidebar.update(cx, |view, cx| view.install_imported_projects(projects, cx));
                cx.notify();
            }
            Some(Err(error)) => shell
                .state
                .set_error(crate::palette::localized_error(&error)),
            None => shell
                .state
                .set_error(rust_i18n::t!("api.project.import_thread_failed").to_string()),
        });
    });
    state.runtime.tasks.background.api_import = Some(task);
}

fn read_import_file(path: &std::path::Path) -> Result<Vec<ApiProject>, transom_core::AppError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        transom_core::AppError::with_detail(
            transom_core::ErrorCode::Io,
            format!(
                "读取 API 项目文件元数据失败，路径: {}，原因: {error}",
                path.display()
            ),
        )
    })?;
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err(transom_core::AppError::with_detail(
            transom_core::ErrorCode::Configuration,
            format!(
                "API 项目导入文件过大，路径: {}，上限: {} 字节",
                path.display(),
                MAX_IMPORT_BYTES
            ),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        transom_core::AppError::with_detail(
            transom_core::ErrorCode::Io,
            format!(
                "读取 API 项目文件失败，路径: {}，原因: {error}",
                path.display()
            ),
        )
    })?;
    let text = String::from_utf8(bytes).map_err(|error| {
        transom_core::AppError::with_detail(
            transom_core::ErrorCode::Configuration,
            format!(
                "API 项目文件不是有效 UTF-8，路径: {}，原因: {error}",
                path.display()
            ),
        )
    })?;
    let format = import_format_for_path(path, &text);
    import_projects(&text, format)
}

fn import_format_for_path(path: &std::path::Path, text: &str) -> ApiImportFormat {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "yaml" | "yml" => ApiImportFormat::OpenApi,
        "curl" | "sh" | "bash" | "txt" => ApiImportFormat::Curl,
        _ => {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
                return ApiImportFormat::Transom;
            };
            if value.get("openapi").is_some() || value.get("swagger").is_some() {
                ApiImportFormat::OpenApi
            } else if value.pointer("/info/name").is_some() && value.get("item").is_some() {
                ApiImportFormat::Postman
            } else {
                ApiImportFormat::Transom
            }
        }
    }
}

fn start_export(
    state: &mut AppState,
    project: ApiProject,
    format: ApiExportFormat,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let (title, extension) = match format {
        ApiExportFormat::Transom => (
            rust_i18n::t!("api.project.export_transom").to_string(),
            "json",
        ),
        ApiExportFormat::Curl => (rust_i18n::t!("api.project.export_curl").to_string(), "sh"),
        ApiExportFormat::Postman => (
            rust_i18n::t!("api.project.export_postman").to_string(),
            "json",
        ),
        ApiExportFormat::OpenApi => (
            rust_i18n::t!("api.project.export_openapi").to_string(),
            "yaml",
        ),
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title(rust_i18n::t!("api.project.export_title").to_string())
        .set_file_name(format!("{}.{}", project.display_name(), extension))
        .add_filter(title, &[extension])
        .save_file()
    else {
        return;
    };
    let task = cx.spawn_in(window, async move |this, cx| {
        let result = crate::app::spawn_blocking_local(move || {
            let content = export_project(&project, format)?;
            std::fs::write(&path, content).map_err(|error| {
                transom_core::AppError::with_detail(
                    transom_core::ErrorCode::Io,
                    format!(
                        "写入 API 项目导出文件失败，路径: {}，原因: {error}",
                        path.display()
                    ),
                )
            })
        })
        .await;
        let _ = this.update_in(cx, |shell, _window, cx| match result {
            Some(Ok(())) => shell
                .state
                .set_transient_status(rust_i18n::t!("api.project.exported").to_string(), cx),
            Some(Err(error)) => shell
                .state
                .set_error(crate::palette::localized_error(&error)),
            None => shell
                .state
                .set_error(rust_i18n::t!("api.project.export_thread_failed").to_string()),
        });
    });
    state.runtime.tasks.background.api_export = Some(task);
}
