//! API 项目工作区实体：请求、环境、认证、断言和协议运行状态。

use super::kv_editor::KvEditorEvent;
use super::workspace_folder::FolderEditor;
pub(crate) use super::workspace_types::{
    ApiAuxiliarySection, ApiBodyMode, ApiRequestTab, ApiResponseTab, ApiWorkspaceEditors,
    ApiWorkspaceEvent, ApiWorkspaceServices,
};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use transom_api::{
    ApiAssertionResult, ApiEnvironmentId, ApiFolderId, ApiImportFormat, ApiMethod, ApiProject,
    ApiProtocol, ApiRequestId, ApiResponseSnapshot, ApiWebSocketHandle, import_projects,
};

/// 单个 API 项目标签的工作区实体。
pub(crate) struct ApiWorkspace {
    pub(crate) project: ApiProject,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) protocol: ApiProtocol,
    pub(crate) environment_id: Option<ApiEnvironmentId>,
    pub(crate) request_tab: ApiRequestTab,
    pub(crate) response_tab: ApiResponseTab,
    pub(crate) selected_history: Option<usize>,
    pub(crate) auxiliary_section: Option<ApiAuxiliarySection>,
    pub(crate) codegen_target: transom_api::CodeTarget,
    pub(crate) editors: ApiWorkspaceEditors,
    pub(crate) response: Option<ApiResponseSnapshot>,
    pub(crate) assertion_results: Vec<ApiAssertionResult>,
    pub(crate) sending: bool,
    pub(crate) ws_connected: bool,
    pub(crate) notice: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) tree_editor_split: Entity<ResizableState>,
    pub(crate) request_response_split: Entity<ResizableState>,
    pub(crate) services: ApiWorkspaceServices,
    pub(crate) send_task: Option<Task<()>>,
    pub(crate) ws_task: Option<Task<()>>,
    pub(crate) ws_event_task: Option<Task<()>>,
    pub(crate) ws_handle: Option<ApiWebSocketHandle>,
    pub(crate) folder_editor: Option<FolderEditor>,
    pub(crate) pending_folder_delete: Option<ApiFolderId>,
    editor_subscriptions: Vec<Subscription>,
    pub(super) binary_file_task: Option<Task<()>>,
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
            request_tab: ApiRequestTab::Query,
            response_tab: ApiResponseTab::Body,
            selected_history: None,
            auxiliary_section: None,
            codegen_target: transom_api::CodeTarget::Curl,
            editors,
            response: None,
            assertion_results: Vec::new(),
            sending: false,
            ws_connected: false,
            notice: None,
            error: None,
            tree_editor_split: cx.new(|_| ResizableState::default()),
            request_response_split: cx.new(|_| ResizableState::default()),
            services,
            send_task: None,
            ws_task: None,
            ws_event_task: None,
            ws_handle: None,
            folder_editor: None,
            pending_folder_delete: None,
            editor_subscriptions: Vec::new(),
            binary_file_task: None,
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
                let super::form_editor::FormEditorEvent::Changed(fields) = event;
                this.apply_form_fields(fields, cx);
            }),
            cx.subscribe_in(&workspace.editors.auth, window, |this, _, event, _, cx| {
                let super::auth_editor::AuthEditorEvent::Changed(auth) = event;
                this.apply_auth(auth.clone(), cx);
            }),
            cx.subscribe_in(
                &workspace.editors.assertions,
                window,
                |this, _, event, _, cx| {
                    let super::assertion_editor::AssertionEditorEvent::Changed(assertions) = event;
                    this.apply_assertions(assertions.clone(), cx);
                },
            ),
            cx.subscribe_in(
                &workspace.editors.environment,
                window,
                |this, _, event, _, cx| {
                    let super::environment_editor::EnvironmentEditorEvent::Changed(variables) =
                        event;
                    this.apply_environment_variables(variables.clone(), cx);
                },
            ),
            cx.subscribe_in(
                &workspace.editors.cookies,
                window,
                |this, _, event, _, cx| {
                    let super::cookie_editor::CookieEditorEvent::Changed(cookies) = event;
                    this.apply_cookies(cookies.clone(), cx);
                },
            ),
        ];
        workspace.load_selected(window, cx);
        workspace.load_environment(window, cx);
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
                self.error = error.detail;
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
        self.error = None;
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
        self.request_tab = tab;
        cx.notify();
    }

    /// 切换响应面板页签；只改变当前工作区的视图状态。
    pub(crate) fn set_response_tab(&mut self, tab: ApiResponseTab, cx: &mut Context<Self>) {
        self.response_tab = tab;
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
        self.auxiliary_section = (self.auxiliary_section != Some(section)).then_some(section);
        cx.notify();
    }

    fn load_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let environment = self.project.active_environment();
        self.environment_id = self.project.active_environment_id.or(Some(environment.id));
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
