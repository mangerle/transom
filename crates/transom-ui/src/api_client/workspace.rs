//! API 项目工作区实体：请求、环境、认证、断言和协议运行状态。

use super::kv_editor::KvEditorEvent;
pub(crate) use super::workspace_types::{
    ApiAuxiliarySection, ApiRequestTab, ApiResponseTab, ApiWorkspaceEditors, ApiWorkspaceEvent,
    ApiWorkspaceServices,
};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use transom_api::{
    ApiAssertionResult, ApiEnvironmentId, ApiImportFormat, ApiMethod, ApiProject, ApiProtocol,
    ApiRequest, ApiRequestId, ApiResponseSnapshot, ApiWebSocketHandle, import_projects,
};

/// 单个 API 项目标签的工作区实体。
pub(crate) struct ApiWorkspace {
    pub(crate) project: ApiProject,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) protocol: ApiProtocol,
    pub(crate) environment_id: Option<ApiEnvironmentId>,
    pub(crate) request_tab: ApiRequestTab,
    pub(crate) editor_ratio: f32,
    pub(crate) response_tab: ApiResponseTab,
    pub(crate) auxiliary_section: Option<ApiAuxiliarySection>,
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
    editor_subscriptions: Vec<Subscription>,
    binary_file_task: Option<Task<()>>,
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
            editor_ratio: 0.5,
            response_tab: ApiResponseTab::Body,
            auxiliary_section: None,
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
            editor_subscriptions: Vec::new(),
            binary_file_task: None,
        };
        workspace.editor_subscriptions = vec![
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
        self.project.upsert_request(request);
        self.load_selected(window, cx);
        self.error = None;
        cx.notify();
        true
    }

    /// 切换 HTTP Body 类型，并同步表单编辑器的文件能力。
    pub(crate) fn set_body_type(
        &mut self,
        body_type: transom_api::ApiBodyType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_editor(cx);
        let Some(request_id) = self.selected_request else {
            return;
        };
        if let Some(request) = self
            .project
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        {
            request.body_type = body_type;
            self.project.touch();
        }
        self.editors.form.update(cx, |editor, cx| {
            editor.set_allow_files(body_type == transom_api::ApiBodyType::Multipart, window, cx);
        });
        cx.notify();
    }

    /// 选择 Binary 请求体文件，路径只在发送时读取。
    pub(crate) fn pick_binary_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self.binary_file_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(request_id) = this.selected_request else {
                    return;
                };
                if let Some(request) = this
                    .project
                    .requests
                    .iter_mut()
                    .find(|request| request.id == request_id)
                {
                    request.body_type = transom_api::ApiBodyType::Binary;
                    request.body_file = format!("file://{}", path.display());
                    this.project.touch();
                    this.editors.form.update(cx, |editor, cx| {
                        editor.set_allow_files(false, window, cx);
                    });
                    cx.notify();
                }
            });
        }));
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
    pub(crate) fn set_editor_ratio(
        &mut self,
        ratio: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sizes = self.request_response_split.read(cx).sizes().clone();
        if sizes.len() < 2 {
            self.editor_ratio = ratio;
            cx.notify();
            return;
        }
        let total = sizes.iter().copied().sum::<Pixels>();
        let target = total * ratio;
        self.request_response_split.update(cx, |state, cx| {
            state.resize_panel(0, target, window, cx);
        });
        self.editor_ratio = ratio;
        cx.notify();
    }

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

    fn load_cookies(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = transom_api::cookies_text(&self.project.cookies);
        self.editors
            .cookies
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
