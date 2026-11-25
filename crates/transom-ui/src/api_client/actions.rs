//! API 客户端窗口动作：打开项目标签、导入文件、持久化与事件落地。

use super::{ApiSidebarEvent, ApiWorkspace, ApiWorkspaceEvent, ApiWorkspaceServices};
use crate::app::{AppShell, AppState, WorkspaceTab};
use gpui_kit::*;
use transom_api::{
    ApiExportFormat, ApiImportFormat, ApiProject, ApiRegistry, MAX_IMPORT_BYTES, export_project,
    import_projects,
};
use transom_core::TabId;

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

/// 打开或聚焦唯一的 API 项目标签；若已存在 API 标签则在标签内直接切换项目。
pub(crate) fn open_api_project(
    state: &mut AppState,
    project: ApiProject,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(existing) = state.workspace.api_tab_for_project(project.id) {
        state.workspace.active_tab = Some(existing);
        state.present_active_group();
        cx.notify();
        return;
    }

    if let Some(existing_tab_id) = state
        .workspace
        .tabs
        .iter()
        .find(|tab| tab.api_workspace().is_some())
        .map(|tab| tab.id)
    {
        let workspace = attach_api_workspace(state, existing_tab_id, &project, window, cx);
        if let Some(tab) = state.workspace.tab_mut(existing_tab_id) {
            tab.replace_api_client(project.display_name().to_string(), project.id, workspace);
        }
        state.workspace.active_tab = Some(existing_tab_id);
        state.present_active_group();
        log::info!(
            "已在 API 标签内切换项目，项目: {}，标签: {}",
            project.display_name(),
            existing_tab_id
        );
        cx.notify();
        return;
    }

    let tab_id = TabId::new();
    let title = project.display_name().to_string();
    let workspace = attach_api_workspace(state, tab_id, &project, window, cx);
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

fn attach_api_workspace(
    state: &mut AppState,
    tab_id: TabId,
    project: &ApiProject,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> Entity<ApiWorkspace> {
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
    workspace
}

/// 聚焦 API 客户端：已有项目标签则激活，否则直接载入已有或自动创建的默认项目。
pub(crate) fn focus_api_client(
    state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    state
        .session
        .sidebar
        .show_section(crate::sidebar::SidebarSection::ApiClient);
    if state
        .workspace
        .active_tab
        .and_then(|tab_id| state.workspace.tab(tab_id))
        .is_some_and(|tab| tab.api_project_id().is_some())
    {
        cx.notify();
        return;
    }
    if let Some(tab_id) = state
        .workspace
        .tabs
        .iter()
        .find_map(|tab| tab.api_project_id().map(|_| tab.id))
    {
        state.workspace.active_tab = Some(tab_id);
        state.present_active_group();
        cx.notify();
        return;
    }
    let project = state
        .features
        .api_sidebar
        .update(cx, |view, cx| view.ensure_default_project(cx));
    open_api_project(state, project, window, cx);
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
                && state.workspace.api_tab_for_project(project_id).is_some()
            {
                let next_project = state
                    .features
                    .api_sidebar
                    .update(cx, |view, cx| view.ensure_default_project(cx));
                open_api_project(state, next_project, window, cx);
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
        ApiWorkspaceEvent::SyncProject { project } => {
            sync_project_snapshot(state, project, cx);
            cx.notify();
        }
        ApiWorkspaceEvent::PersistProject { project, notice } => {
            let registry = sync_project_snapshot(state, project, cx);
            if let Err(error) = state.runtime.persistence.save_api_registry(&registry) {
                state.set_error(error.detail.unwrap_or_else(|| format!("{:?}", error.code)));
            }
            if let Some(notice) = notice {
                state.set_transient_status(notice, cx);
            }
            if state.workspace.tab(tab_id).is_some() {
                cx.notify();
            }
        }
    }
}

fn sync_project_snapshot(
    state: &mut AppState,
    project: ApiProject,
    cx: &mut Context<AppShell>,
) -> ApiRegistry {
    let sidebar = state.features.api_sidebar.clone();
    let registry = sidebar.update(cx, |view, cx| {
        view.apply_project_snapshot(project.clone(), cx);
        view.state.registry.clone()
    });
    update_open_project(state, &project, cx);
    registry
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
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("API project")
        .to_string();
    let sidebar = state.features.api_sidebar.clone();
    sidebar.update(cx, |view, cx| {
        view.begin_import(file_name.clone(), cx);
    });

    let task = cx.spawn_in(window, async move |this, cx| {
        let parsed = crate::app::spawn_blocking_local(move || read_import_file(&path)).await;
        let _ = this.update_in(cx, |shell, _window, cx| match parsed {
            Some(Ok(parsed)) => {
                let sidebar = shell.state.features.api_sidebar.clone();
                sidebar.update(cx, |view, cx| {
                    view.preview_import(parsed.projects, parsed.format, file_name, cx)
                });
                cx.notify();
            }
            Some(Err(error)) => {
                let sidebar = shell.state.features.api_sidebar.clone();
                sidebar.update(cx, |view, cx| view.cancel_import(cx));
                shell
                    .state
                    .set_error(crate::palette::localized_error(&error));
            }
            None => {
                let sidebar = shell.state.features.api_sidebar.clone();
                sidebar.update(cx, |view, cx| view.cancel_import(cx));
                shell
                    .state
                    .set_error(rust_i18n::t!("api.project.import_thread_failed").to_string());
            }
        });
    });
    state.runtime.tasks.background.api_import = Some(task);
}

struct ParsedImport {
    projects: Vec<ApiProject>,
    format: ApiImportFormat,
}

fn read_import_file(path: &std::path::Path) -> Result<ParsedImport, transom_core::AppError> {
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
    let projects = import_projects(&text, format)?;
    Ok(ParsedImport { projects, format })
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
