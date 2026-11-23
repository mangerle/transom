//! API 项目工作区实体：接口树、请求编辑器、发送任务与响应状态。

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use transom_api::{
    ApiHttpService, ApiMethod, ApiProject, ApiRequest, ApiRequestId, ApiResponseSnapshot,
};

/// API 工作区需要外壳处理的窗口级意图。
#[derive(Clone)]
pub(crate) enum ApiWorkspaceEvent {
    /// 项目快照已变化，请求由外壳更新登记表并异步落盘。
    PersistProject {
        project: ApiProject,
        notice: Option<String>,
    },
}

/// 单个 API 项目标签的工作区实体。
///
/// 设计原理（Why）：项目打开后，接口树选择、草稿、响应与在途发送任务都应
/// 随标签释放；实体只给侧栏发出完整项目快照，外壳不回读工作区，避免 GPUI
/// 事件回调期间再次租借正在派发事件的实体。
pub(crate) struct ApiWorkspace {
    pub(crate) project: ApiProject,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) request_name: Entity<InputState>,
    pub(crate) request_url: Entity<InputState>,
    pub(crate) request_headers: Entity<TextareaState>,
    pub(crate) request_body: Entity<TextareaState>,
    pub(crate) response_headers: Entity<TextareaState>,
    pub(crate) response_body: Entity<TextareaState>,
    pub(crate) response: Option<ApiResponseSnapshot>,
    pub(crate) sending: bool,
    pub(crate) error: Option<String>,
    pub(crate) tree_editor_split: Entity<ResizableState>,
    pub(crate) request_response_split: Entity<ResizableState>,
    http: ApiHttpService,
    send_task: Option<Task<()>>,
}

impl EventEmitter<ApiWorkspaceEvent> for ApiWorkspace {}

impl ApiWorkspace {
    /// 构造工作区并载入项目第一个请求的草稿。
    pub(crate) fn new(
        project: ApiProject,
        http: ApiHttpService,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let request_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.name_placeholder").to_string())
        });
        let request_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.url_placeholder").to_string())
        });
        let request_headers = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.headers_placeholder").to_string())
        });
        let request_body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.body_placeholder").to_string())
        });
        let response_headers = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(rust_i18n::t!("api.response.headers_placeholder").to_string())
        });
        let response_body = cx.new(|cx| TextareaState::new(window, cx));
        let mut workspace = Self {
            selected_request: project.requests.first().map(|request| request.id),
            project,
            method: ApiMethod::Get,
            request_name,
            request_url,
            request_headers,
            request_body,
            response_headers,
            response_body,
            response: None,
            sending: false,
            error: None,
            tree_editor_split: cx.new(|_| ResizableState::default()),
            request_response_split: cx.new(|_| ResizableState::default()),
            http,
            send_task: None,
        };
        workspace.load_selected(window, cx);
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

    /// 更新当前请求方法。
    pub(crate) fn set_method(&mut self, method: ApiMethod, cx: &mut Context<Self>) {
        self.method = method;
        self.capture_editor(cx);
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

    /// 发送当前请求；新的发送会丢弃上一个 Future，旧请求随之中止。
    pub(crate) fn send_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.capture_editor(cx) {
            cx.notify();
            return;
        }
        let Some(mut request) = self.selected_request().cloned() else {
            self.error = Some(rust_i18n::t!("api.request.select_first").to_string());
            cx.notify();
            return;
        };
        if request.url.trim().is_empty() {
            self.error = Some(rust_i18n::t!("api.request.url_required").to_string());
            cx.notify();
            return;
        }
        request.url = request.resolved_url(&self.project.base_url);

        self.sending = true;
        self.response = None;
        self.response_headers
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.response_body
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.error = None;
        cx.emit(ApiWorkspaceEvent::PersistProject {
            project: self.project.clone(),
            notice: None,
        });
        cx.notify();

        let http = self.http.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let mut exchange = http.start(request);
            let result = exchange.recv().await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = false;
                match result {
                    Ok(response) => {
                        let headers = response
                            .headers
                            .iter()
                            .map(|(name, value)| format!("{name}: {value}"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        this.response_headers.update(cx, |state, cx| {
                            state.set_value(headers, window, cx);
                        });
                        this.response_body.update(cx, |state, cx| {
                            state.set_value(response.body.clone(), window, cx);
                        });
                        this.response = Some(response);
                        this.error = None;
                    }
                    Err(error) => {
                        this.response = None;
                        this.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        });
        self.send_task = Some(task);
    }

    /// 外壳保存项目后回填工作区，保持标题与侧栏一致。
    pub(crate) fn apply_project_snapshot(&mut self, project: ApiProject, cx: &mut Context<Self>) {
        self.project = project;
        self.selected_request = self
            .selected_request
            .filter(|request_id| self.project.request(*request_id).is_some())
            .or_else(|| self.project.requests.first().map(|request| request.id));
        cx.notify();
    }

    fn capture_editor(&mut self, cx: &App) -> bool {
        let Some(request_id) = self.selected_request else {
            return true;
        };
        let Some(mut request) = self.project.request(request_id).cloned() else {
            return false;
        };
        request.name = self.request_name.read(cx).value().trim().to_string();
        request.url = self.request_url.read(cx).value().trim().to_string();
        request.body = self.request_body.read(cx).value().to_string();
        let header_text = self.request_headers.read(cx).value().to_string();
        if let Err(error) = request.set_header_text(&header_text) {
            self.error = Some(error.to_string());
            return false;
        }
        request.method = self.method;
        self.error = None;
        self.project.upsert_request(request);
        true
    }

    fn load_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.selected_request().cloned();
        let (name, url, headers, body, method) = request.map_or_else(
            || {
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    ApiMethod::Get,
                )
            },
            |request| {
                let headers = request.header_text();
                (
                    request.display_name().to_string(),
                    request.url,
                    headers,
                    request.body,
                    request.method,
                )
            },
        );
        self.method = method;
        self.request_name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.request_url
            .update(cx, |state, cx| state.set_value(url, window, cx));
        self.request_headers
            .update(cx, |state, cx| state.set_value(headers, window, cx));
        self.request_body
            .update(cx, |state, cx| state.set_value(body, window, cx));
    }
}
