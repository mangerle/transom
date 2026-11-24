//! API 项目工作区实体：请求、环境、认证、断言和协议运行状态。

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use transom_api::{
    ApiAssertionResult, ApiEnvironmentId, ApiHttpService, ApiMethod, ApiMockHandle, ApiMockService,
    ApiProject, ApiProtocol, ApiRequest, ApiRequestId, ApiResponseSnapshot, ApiWebSocketHandle,
    ApiWebSocketService,
};

/// API 工作区需要的外壳服务集合。
#[derive(Clone)]
pub(crate) struct ApiWorkspaceServices {
    pub(crate) http: ApiHttpService,
    pub(crate) websocket: ApiWebSocketService,
    pub(crate) mock: ApiMockService,
}

/// 左侧树/右侧编辑区共享的高级面板页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiAuxiliarySection {
    Environment,
    Cookies,
    Auth,
    Assertions,
    Mock,
    History,
}

/// API 工作区需要外壳处理的窗口级意图。
#[derive(Clone)]
pub(crate) enum ApiWorkspaceEvent {
    /// 项目快照已变化，请求由外壳更新登记表并异步落盘。
    PersistProject {
        project: ApiProject,
        notice: Option<String>,
    },
}

/// 所有可编辑文本实体集中在一个参数对象中，避免渲染函数反查工作区实体。
pub(crate) struct ApiWorkspaceEditors {
    pub(crate) request_name: Entity<InputState>,
    pub(crate) request_url: Entity<InputState>,
    pub(crate) request_headers: Entity<TextareaState>,
    pub(crate) request_body: Entity<TextareaState>,
    pub(crate) grpc_service: Entity<InputState>,
    pub(crate) grpc_method: Entity<InputState>,
    pub(crate) graphql_variables: Entity<TextareaState>,
    pub(crate) auth: Entity<TextareaState>,
    pub(crate) assertions: Entity<TextareaState>,
    pub(crate) environment: Entity<TextareaState>,
    pub(crate) cookies: Entity<TextareaState>,
    pub(crate) mock: Entity<TextareaState>,
    pub(crate) history: Entity<TextareaState>,
    pub(crate) websocket_input: Entity<TextareaState>,
    pub(crate) websocket_log: Entity<TextareaState>,
    pub(crate) response_headers: Entity<TextareaState>,
    pub(crate) response_body: Entity<TextareaState>,
}

/// 单个 API 项目标签的工作区实体。
pub(crate) struct ApiWorkspace {
    pub(crate) project: ApiProject,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) protocol: ApiProtocol,
    pub(crate) environment_id: Option<ApiEnvironmentId>,
    pub(crate) auxiliary_section: Option<ApiAuxiliarySection>,
    pub(crate) editors: ApiWorkspaceEditors,
    pub(crate) response: Option<ApiResponseSnapshot>,
    pub(crate) assertion_results: Vec<ApiAssertionResult>,
    pub(crate) sending: bool,
    pub(crate) ws_connected: bool,
    pub(crate) mock_port: Option<u16>,
    pub(crate) notice: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) tree_editor_split: Entity<ResizableState>,
    pub(crate) request_response_split: Entity<ResizableState>,
    pub(crate) services: ApiWorkspaceServices,
    pub(crate) send_task: Option<Task<()>>,
    pub(crate) ws_task: Option<Task<()>>,
    pub(crate) ws_event_task: Option<Task<()>>,
    pub(crate) ws_handle: Option<ApiWebSocketHandle>,
    pub(crate) mock_task: Option<Task<()>>,
    pub(crate) mock_handle: Option<ApiMockHandle>,
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
        let editors = super::workspace_editors::new_editors(window, cx);
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
            auxiliary_section: None,
            editors,
            response: None,
            assertion_results: Vec::new(),
            sending: false,
            ws_connected: false,
            mock_port: None,
            notice: None,
            error: None,
            tree_editor_split: cx.new(|_| ResizableState::default()),
            request_response_split: cx.new(|_| ResizableState::default()),
            services,
            send_task: None,
            ws_task: None,
            ws_event_task: None,
            ws_handle: None,
            mock_task: None,
            mock_handle: None,
        };
        workspace.load_selected(window, cx);
        workspace.load_environment(window, cx);
        workspace.load_cookies(window, cx);
        workspace.load_mock(window, cx);
        workspace.refresh_history(window, cx);
        workspace
    }

    /// 当前选中请求的 clone；没有选中请求时返回 `None`。
    pub(crate) fn selected_request(&self) -> Option<&ApiRequest> {
        self.selected_request
            .and_then(|request_id| self.project.request(request_id))
    }

    /// 选择项目树中的请求，并把当前草稿先写回项目。
    pub(crate) fn select_request(
        &mut self,
        request_id: ApiRequestId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_request == Some(request_id) {
            return;
        }
        self.capture_editor(cx);
        if self.project.request(request_id).is_none() {
            return;
        }
        self.selected_request = Some(request_id);
        self.load_selected(window, cx);
        cx.notify();
    }

    /// 新建请求并立即选中；保存动作仍由用户显式触发。
    pub(crate) fn new_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_editor(cx);
        let folder_id = self
            .selected_request()
            .and_then(|request| request.folder_id);
        let request = ApiRequest::new(
            rust_i18n::t!("api.request.default_name").to_string(),
            folder_id,
        );
        let request_id = request.id;
        self.project.upsert_request(request);
        self.selected_request = Some(request_id);
        self.load_selected(window, cx);
        cx.notify();
    }

    /// 删除当前请求并选中下一个可用请求。
    pub(crate) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request_id) = self.selected_request else {
            return;
        };
        if !self.project.remove_request(request_id) {
            return;
        }
        self.selected_request = self.project.requests.first().map(|request| request.id);
        self.load_selected(window, cx);
        cx.notify();
    }

    /// 更新当前请求方法和协议。
    pub(crate) fn set_method(&mut self, method: ApiMethod, cx: &mut Context<Self>) {
        self.method = method;
        self.capture_editor(cx);
        cx.notify();
    }

    /// 设置协议并同步请求草稿。
    pub(crate) fn set_protocol(&mut self, protocol: ApiProtocol, cx: &mut Context<Self>) {
        self.protocol = protocol;
        self.capture_editor(cx);
        cx.notify();
    }

    /// 切换当前环境并刷新变量编辑区。
    pub(crate) fn set_environment(
        &mut self,
        environment_id: ApiEnvironmentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_environment(cx);
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
        self.auxiliary_section = (self.auxiliary_section != Some(section)).then_some(section);
        cx.notify();
    }

    pub(super) fn capture_editor(&mut self, cx: &App) -> bool {
        let Some(request_id) = self.selected_request else {
            return true;
        };
        let Some(mut request) = self.project.request(request_id).cloned() else {
            return false;
        };
        request.name = self
            .editors
            .request_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        request.url = self.editors.request_url.read(cx).value().trim().to_string();
        request.body = self.editors.request_body.read(cx).value().to_string();
        request.grpc_service = self
            .editors
            .grpc_service
            .read(cx)
            .value()
            .trim()
            .to_string();
        request.grpc_method = self.editors.grpc_method.read(cx).value().trim().to_string();
        request.protocol = self.protocol;
        request.method = self.method;
        let header_text = self.editors.request_headers.read(cx).value().to_string();
        if let Err(error) = request.set_header_text(&header_text) {
            self.error = Some(error.to_string());
            return false;
        }
        request.graphql_variables = self.editors.graphql_variables.read(cx).value().to_string();
        let auth_text = self.editors.auth.read(cx).value().to_string();
        if let Err(error) = request.set_auth_text(&auth_text) {
            self.error = Some(error.to_string());
            return false;
        }
        let assertions_text = self.editors.assertions.read(cx).value().to_string();
        if let Err(error) = request.set_assertions_text(&assertions_text) {
            self.error = Some(error.to_string());
            return false;
        }
        self.project.upsert_request(request);
        self.capture_environment(cx);
        self.capture_cookies(cx);
        self.error = None;
        true
    }

    fn load_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.selected_request().cloned();
        let (
            name,
            url,
            headers,
            body,
            method,
            protocol,
            graphql,
            auth,
            assertions,
            grpc_service,
            grpc_method,
        ) = request.map_or_else(
            || {
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    ApiMethod::Get,
                    ApiProtocol::Http,
                    "{}".to_string(),
                    "{\"kind\":\"none\"}".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            },
            |request| {
                let headers = request.header_text();
                let auth = request.auth_text();
                let assertions = request.assertions_text();
                (
                    request.display_name().to_string(),
                    request.url,
                    headers,
                    request.body,
                    request.method,
                    request.protocol,
                    request.graphql_variables,
                    auth,
                    assertions,
                    request.grpc_service,
                    request.grpc_method,
                )
            },
        );
        self.method = method;
        self.protocol = protocol;
        self.editors
            .request_name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.editors
            .request_url
            .update(cx, |state, cx| state.set_value(url, window, cx));
        self.editors
            .request_headers
            .update(cx, |state, cx| state.set_value(headers, window, cx));
        self.editors
            .request_body
            .update(cx, |state, cx| state.set_value(body, window, cx));
        self.editors
            .grpc_service
            .update(cx, |state, cx| state.set_value(grpc_service, window, cx));
        self.editors
            .grpc_method
            .update(cx, |state, cx| state.set_value(grpc_method, window, cx));
        self.editors
            .graphql_variables
            .update(cx, |state, cx| state.set_value(graphql, window, cx));
        self.editors
            .auth
            .update(cx, |state, cx| state.set_value(auth, window, cx));
        self.editors
            .assertions
            .update(cx, |state, cx| state.set_value(assertions, window, cx));
    }

    fn load_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let environment = self.project.active_environment();
        self.environment_id = self.project.active_environment_id.or(Some(environment.id));
        self.editors.environment.update(cx, |state, cx| {
            state.set_value(environment.variables_text(), window, cx)
        });
    }

    fn load_cookies(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = transom_api::cookies_text(&self.project.cookies);
        self.editors
            .cookies
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn load_mock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = serde_json::to_string_pretty(&self.project.mock_routes)
            .unwrap_or_else(|_| "[]".to_string());
        self.editors
            .mock
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    pub(crate) fn refresh_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .project
            .history
            .iter()
            .rev()
            .map(|entry| {
                rust_i18n::t!(
                    "api.history.line",
                    method = entry.method,
                    url = entry.url,
                    status = entry.status,
                    elapsed = entry.elapsed_millis,
                    passed = entry.assertions_passed,
                    total = entry.assertions_total
                )
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.editors
            .history
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    pub(crate) fn environment(&self) -> transom_api::ApiEnvironment {
        self.environment_id
            .and_then(|id| {
                self.project
                    .environments
                    .iter()
                    .find(|environment| environment.id == id)
            })
            .cloned()
            .unwrap_or_else(|| self.project.active_environment())
    }
}
