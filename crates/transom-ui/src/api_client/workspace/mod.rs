//! API 项目工作区实体：请求、环境、认证、断言和协议运行状态。

mod actions;
mod advanced_view;
mod body_actions;
mod body_view;
mod editor_state;
mod editors;
mod folder;
mod global_config;
pub(in crate::api_client) mod global_params_view;
mod history_view;
mod kv_sync;
mod project_switcher;
mod request_actions;
mod request_view;
mod response_tables;
mod response_view;
mod save_menu;
mod tabs_bar;
mod tree_view;
mod types;
mod view;

use super::editors::kv::KvEditorEvent;
use super::sidebar::{ApiSidebarState, ApiSidebarView};
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use transom_api::{
    ApiEnvironmentId, ApiImportFormat, ApiMethod, ApiProject, ApiProtocol, ApiRequestId,
    import_projects,
};
pub(crate) use types::{
    ApiAuxiliarySection, ApiBodyMode, ApiConfigModalSection, ApiGlobalParameterTab, ApiRequestTab,
    ApiResponseTab, ApiWorkspaceEditors, ApiWorkspaceEvent, ApiWorkspaceServices,
};
use types::{ExecutionState, WorkspaceSplits, WorkspaceUiState};
pub(crate) use view::{ApiWorkspaceParams, render_api_workspace};

/// 单个 API 项目标签的工作区实体。
pub(crate) struct ApiWorkspace {
    pub(crate) project: ApiProject,
    pub(in crate::api_client) selected_request: Option<ApiRequestId>,
    pub(in crate::api_client) method: ApiMethod,
    pub(in crate::api_client) protocol: ApiProtocol,
    pub(in crate::api_client) environment_id: Option<ApiEnvironmentId>,
    pub(in crate::api_client) ui: WorkspaceUiState,
    pub(in crate::api_client) exec: ExecutionState,
    pub(in crate::api_client) editors: ApiWorkspaceEditors,
    pub(in crate::api_client) splits: WorkspaceSplits,
    services: ApiWorkspaceServices,
    editor_subscriptions: Vec<Subscription>,
}

impl EventEmitter<ApiWorkspaceEvent> for ApiWorkspace {}

impl ApiWorkspace {
    /// 构造工作区并载入项目第一个请求与环境。
    pub(crate) fn new(
        project: ApiProject,
        services: ApiWorkspaceServices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editors = editors::new_editors(window, cx);
        let active_environment_id = project.active_environment_id;
        let mut workspace = Self {
            selected_request: project.requests.first().map(|request| request.id),
            protocol: project
                .requests
                .first()
                .map_or(ApiProtocol::Http, |request| request.protocol),
            environment_id: active_environment_id,
            project,
            method: ApiMethod::Get,
            ui: WorkspaceUiState::default(),
            exec: ExecutionState::default(),
            editors,
            splits: WorkspaceSplits::new(cx),
            services,
            editor_subscriptions: Vec::new(),
        };
        workspace.editor_subscriptions = vec![
            cx.subscribe_in(
                &workspace.editors.request_name,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.apply_request_name(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &workspace.editors.save_destination_search,
                window,
                |_this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &workspace.editors.environment_name,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.apply_environment_name(cx);
                    }
                },
            ),
            cx.subscribe_in(&workspace.editors.query, window, |this, _, event, _, cx| {
                let KvEditorEvent::Changed(entries) = event;
                this.apply_query_entries(entries, cx);
            }),
            cx.subscribe_in(
                &workspace.editors.headers,
                window,
                |this, _, event, _, cx| {
                    let KvEditorEvent::Changed(entries) = event;
                    this.apply_header_entries(entries, cx);
                },
            ),
            cx.subscribe_in(&workspace.editors.form, window, |this, _, event, _, cx| {
                let crate::api_client::editors::form::FormEditorEvent::Changed(fields) = event;
                this.apply_form_fields(fields, cx);
            }),
            cx.subscribe_in(&workspace.editors.auth, window, |this, _, event, _, cx| {
                let crate::api_client::editors::auth::AuthEditorEvent::Changed(auth) = event;
                this.apply_auth(auth.clone(), cx);
            }),
            cx.subscribe_in(
                &workspace.editors.assertions,
                window,
                |this, _, event, _, cx| {
                    let crate::api_client::editors::assertion::AssertionEditorEvent::Changed(
                        assertions,
                    ) = event;
                    this.apply_assertions(assertions.clone(), cx);
                },
            ),
            cx.subscribe_in(
                &workspace.editors.environment,
                window,
                |this, _, event, _, cx| {
                    let crate::api_client::editors::environment::EnvironmentEditorEvent::Changed(
                        variables,
                    ) = event;
                    this.apply_environment_variables(variables.clone(), cx);
                },
            ),
            cx.subscribe_in(
                &workspace.editors.cookies,
                window,
                |this, _, event, _, cx| {
                    let crate::api_client::editors::cookie::CookieEditorEvent::Changed(cookies) =
                        event;
                    this.apply_cookies(cookies.clone(), cx);
                },
            ),
            cx.subscribe_in(
                &workspace.editors.global_variables,
                window,
                |this, _, event, _, cx| {
                    let crate::api_client::editors::environment::EnvironmentEditorEvent::Changed(
                        variables,
                    ) = event;
                    this.apply_global_variables(variables.clone(), cx);
                },
            ),
            cx.subscribe_in(
                &workspace.editors.global_query,
                window,
                |this, _, event, _, cx| {
                    let KvEditorEvent::Changed(entries) = event;
                    this.apply_global_parameters(
                        transom_api::ApiGlobalParameterTarget::Query,
                        entries,
                        cx,
                    );
                },
            ),
            cx.subscribe_in(
                &workspace.editors.global_headers,
                window,
                |this, _, event, _, cx| {
                    let KvEditorEvent::Changed(entries) = event;
                    this.apply_global_parameters(
                        transom_api::ApiGlobalParameterTarget::Header,
                        entries,
                        cx,
                    );
                },
            ),
        ];
        workspace.load_selected(window, cx);
        workspace.load_environment(window, cx);
        workspace.load_global_config(window, cx);
        workspace.load_cookies(window, cx);
        workspace
    }

    /// 将粘贴的 cURL 命令填充到当前请求；不是 cURL 时返回 false 交给输入框。
    pub(crate) fn import_curl(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = text.trim();
        if !text.starts_with("curl ") {
            return false;
        }
        if self.selected_request.is_none() {
            return true;
        }
        let imported = match import_projects(text, ApiImportFormat::Curl) {
            Ok(mut projects) => projects
                .pop()
                .and_then(|mut project| project.requests.pop()),
            Err(error) => {
                self.exec.error = error.detail;
                cx.notify();
                return true;
            }
        };
        let Some(imported) = imported else {
            return true;
        };
        let Some(existing_id) = self.selected_request else {
            return true;
        };
        let Some(existing) = self.project.request(existing_id).cloned() else {
            return true;
        };
        let mut request = imported;
        request.id = existing.id;
        request.folder_id = existing.folder_id;
        request.name = existing.name;
        request.raw_language = request
            .body_type
            .raw_language()
            .unwrap_or(request.raw_language);
        self.project.upsert_request(request);
        self.load_selected(window, cx);
        self.exec.error = None;
        cx.notify();
        true
    }

    /// 同时切换协议和方法，避免一次选择触发两次草稿回写。
    pub(crate) fn set_request_mode(
        &mut self,
        protocol: ApiProtocol,
        method: ApiMethod,
        cx: &mut Context<Self>,
    ) {
        self.protocol = protocol;
        self.method = method;
        self.capture_editor(cx);
        cx.notify();
    }

    /// 切换请求编辑页签；只改变当前工作区的视图状态。
    pub(crate) fn set_request_tab(&mut self, tab: ApiRequestTab, cx: &mut Context<Self>) {
        self.ui.request_tab = tab;
        cx.notify();
    }

    /// 切换响应面板页签；只改变当前工作区的视图状态。
    pub(crate) fn set_response_tab(&mut self, tab: ApiResponseTab, cx: &mut Context<Self>) {
        self.ui.response_tab = tab;
        cx.notify();
    }

    /// 按预设比例调整请求区与响应区高度。
    /// 切换当前环境并刷新变量编辑区。
    pub(crate) fn set_environment(
        &mut self,
        environment_id: ApiEnvironmentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.environment_id = Some(environment_id);
        self.project.active_environment_id = Some(environment_id);
        self.load_environment(window, cx);
        cx.notify();
    }

    /// 保存当前项目，发出完整快照供外壳更新侧栏与持久化。
    pub(crate) fn save_project(&mut self, cx: &mut Context<Self>) {
        if !self.capture_editor(cx) {
            cx.notify();
            return;
        }
        cx.emit(ApiWorkspaceEvent::PersistProject {
            project: self.project.clone(),
            notice: Some(rust_i18n::t!("api.project.saved").to_string()),
        });
        cx.notify();
    }

    /// 外壳保存项目后回填工作区，保持标题与侧栏一致。
    pub(crate) fn apply_project_snapshot(&mut self, project: ApiProject, cx: &mut Context<Self>) {
        self.project = project;
        self.environment_id = self.project.active_environment_id;
        self.selected_request = self
            .selected_request
            .filter(|request_id| self.project.request(*request_id).is_some())
            .or_else(|| self.project.requests.first().map(|request| request.id));
        cx.notify();
    }

    /// 打开/关闭高级面板。
    pub(crate) fn toggle_auxiliary(
        &mut self,
        section: ApiAuxiliarySection,
        cx: &mut Context<Self>,
    ) {
        self.ui.auxiliary_section = (self.ui.auxiliary_section != Some(section)).then_some(section);
        cx.notify();
    }

    pub(super) fn load_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let environment = self.project.active_environment();
        self.environment_id = self.project.active_environment_id.or(Some(environment.id));
        self.editors.environment_name.update(cx, |input, cx| {
            input.set_value(environment.name.clone(), window, cx)
        });
        self.editors.environment.update(cx, |editor, cx| {
            editor.set_variables(&environment.variables, window, cx)
        });
    }

    pub(super) fn load_cookies(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editors.cookies.update(cx, |editor, cx| {
            editor.set_cookies(&self.project.cookies, window, cx)
        });
    }

    pub(crate) fn environment(&self) -> transom_api::ApiEnvironment {
        let environment = self
            .environment_id
            .and_then(|id| {
                self.project
                    .environments
                    .iter()
                    .find(|environment| environment.id == id)
            })
            .cloned()
            .unwrap_or_else(|| self.project.active_environment());
        self.project.merged_environment(&environment)
    }
}
