//! API 工作区的发送、WebSocket 与代码生成动作。

use super::ApiWorkspace;
use gpui_kit::*;
use transom_api::{
    ApiEnvironment, ApiHistoryEntry, ApiProtocol, ApiRequest, ApiWebSocketEvent, CodeTarget,
    evaluate_assertions, generate_code, method_for_request, response_excerpt,
};

impl ApiWorkspace {
    /// 发送当前请求；WebSocket 请求改为建立连接。
    pub(crate) fn send_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.capture_request(cx) {
            cx.notify();
            return;
        }
        if self.current_protocol() == ApiProtocol::WebSocket {
            self.connect_websocket(window, cx);
            return;
        }
        let Some(request) = self.selected_request().cloned() else {
            self.set_error(rust_i18n::t!("api.request.select_first").to_string(), cx);
            return;
        };
        if request.url.trim().is_empty() {
            self.set_error(rust_i18n::t!("api.request.url_required").to_string(), cx);
            return;
        }
        let environment = self.environment();
        let resolved = self.project.resolved_request(&request, &environment);
        if let Err(error) = resolved.validate() {
            self.set_error(error.to_string(), cx);
            return;
        }
        let cookies = self.project.cookies.clone();
        self.exec.sending = true;
        self.exec.response = None;
        self.exec.assertion_results.clear();
        self.clear_response_editor(window, cx);
        self.exec.error = None;
        cx.notify();

        let http = self.services.http.clone();
        let project_base = self.project.base_url.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let mut exchange = http.start(resolved.clone(), environment.clone(), cookies);
            let result = exchange.recv().await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.exec.sending = false;
                match result {
                    Ok(response) => this.apply_response(
                        &resolved,
                        &environment,
                        &project_base,
                        response,
                        window,
                        cx,
                    ),
                    Err(error) => {
                        this.exec.response = None;
                        this.exec.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        });
        self.exec.send_task = Some(task);
    }

    /// 重放历史记录对应的当前请求；请求已被删除时给出明确错误。
    pub(crate) fn replay_history(
        &mut self,
        history_index: usize,
        request_id: transom_api::ApiRequestId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.request(request_id).is_none() {
            self.set_error(
                rust_i18n::t!("api.response.history_request_missing").to_string(),
                cx,
            );
            return;
        }
        if history_index < self.project.history.len() {
            self.ui.selected_history = Some(history_index);
        }
        self.select_request(request_id, window, cx);
        self.send_request(window, cx);
    }

    /// 选中历史记录并恢复对应请求；再次点击同一记录会收起详情。
    pub(crate) fn select_history(
        &mut self,
        history_index: usize,
        request_id: transom_api::ApiRequestId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if history_index >= self.project.history.len() {
            return;
        }
        if self.ui.selected_history == Some(history_index) {
            self.ui.selected_history = None;
            cx.notify();
            return;
        }
        self.ui.selected_history = Some(history_index);
        self.select_request(request_id, window, cx);
        cx.notify();
    }

    pub(crate) fn clear_history_selection(&mut self, cx: &mut Context<Self>) {
        self.ui.selected_history = None;
        cx.notify();
    }

    /// 清空当前项目的请求历史并持久化。
    pub(crate) fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.project.history.clear();
        self.ui.selected_history = None;
        self.project.touch();
        self.save_project(cx);
        cx.notify();
    }

    /// 复制当前响应正文；WebSocket 请求复制消息日志。
    pub(crate) fn copy_response_body(&mut self, cx: &mut Context<Self>) {
        let text = if self.current_protocol() == ApiProtocol::WebSocket {
            self.editors.websocket_log.read(cx).value().to_string()
        } else {
            self.editors.response_body.read(cx).value().to_string()
        };
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.set_notice(rust_i18n::t!("api.response.body_copied").to_string(), cx);
    }

    /// 连接 WebSocket，并把收到的消息持续写入日志。
    pub(crate) fn connect_websocket(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.capture_request(cx) {
            return;
        }
        let Some(request) = self.selected_request().cloned() else {
            self.set_error(rust_i18n::t!("api.request.select_first").to_string(), cx);
            return;
        };
        if request.url.trim().is_empty() {
            self.set_error(rust_i18n::t!("api.request.url_required").to_string(), cx);
            return;
        }
        let environment = self.environment();
        let resolved = self.project.resolved_request(&request, &environment);
        let url = resolved.url.clone();
        let headers = resolved.headers.clone();
        // 建立新连接前先关闭并清理旧连接及事件监听任务，避免残留事件流交叉写入日志。
        self.reset_websocket_connection();
        let service = self.services.websocket.clone();
        let start = service.start(url, headers, environment, resolved.auth.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = start.into_handle().await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(mut handle) => {
                    this.reset_websocket_connection();
                    let events = handle.take_events();
                    this.exec.ws_handle = Some(handle);
                    this.exec.ws_connected = true;
                    this.append_ws_log(
                        rust_i18n::t!("api.websocket.connected").to_string(),
                        window,
                        cx,
                    );
                    this.exec.ws_event_task = Some(this.spawn_ws_events(events, window, cx));
                }
                Err(error) => this.set_error(error, cx),
            });
        });
        self.exec.ws_task = Some(task);
    }

    fn reset_websocket_connection(&mut self) {
        if let Some(handle) = self.exec.ws_handle.take()
            && let Err(error) = handle.close()
        {
            log::warn!("关闭旧 WebSocket 连接失败，原因: {error}");
        }
        self.exec.ws_task = None;
        self.exec.ws_event_task = None;
        self.exec.ws_connected = false;
    }

    /// 发送一条 WebSocket 文本消息。
    pub(crate) fn send_websocket_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editors.websocket_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some(handle) = &self.exec.ws_handle else {
            self.set_error(rust_i18n::t!("api.websocket.not_connected").to_string(), cx);
            return;
        };
        if let Err(error) = handle.try_send_text(text.clone()) {
            self.set_error(error, cx);
            return;
        }
        self.editors
            .websocket_input
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        self.set_notice(
            rust_i18n::t!("api.websocket.message_sent", text = text).to_string(),
            cx,
        );
    }

    /// 关闭 WebSocket。
    pub(crate) fn close_websocket(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.exec.ws_handle.take()
            && let Err(error) = handle.close()
        {
            self.exec.ws_handle = Some(handle);
            self.set_error(error, cx);
            return;
        }
        self.exec.ws_task = None;
        self.exec.ws_event_task = None;
        self.exec.ws_connected = false;
        self.set_notice(rust_i18n::t!("api.websocket.closed").to_string(), cx);
    }

    /// 打开当前请求的代码预览。
    pub(crate) fn open_code_preview(
        &mut self,
        target: CodeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.capture_request(cx) {
            return;
        }
        self.ui.codegen_target = target;
        self.regenerate_code_preview(window, cx);
        self.ui.auxiliary_section = Some(super::ApiAuxiliarySection::Code);
        cx.notify();
    }

    /// 切换代码目标并刷新预览。
    pub(crate) fn set_codegen_target(
        &mut self,
        target: CodeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.codegen_target = target;
        self.regenerate_code_preview(window, cx);
        cx.notify();
    }

    /// 将预览中的代码复制到剪贴板。
    pub(crate) fn copy_code_preview(&mut self, cx: &mut Context<Self>) {
        let code = self.editors.code_preview.read(cx).value().to_string();
        if code.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(code));
        self.set_notice(rust_i18n::t!("api.code.copied").to_string(), cx);
    }

    fn regenerate_code_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.selected_request().cloned() else {
            return;
        };
        let request = self.project.resolved_request(&request, &self.environment());
        let code = generate_code(self.ui.codegen_target, &request);
        self.editors
            .code_preview
            .update(cx, |editor, cx| editor.set_value(code, window, cx));
    }

    fn capture_request(&mut self, cx: &App) -> bool {
        self.capture_editor(cx)
    }

    fn apply_response(
        &mut self,
        request: &ApiRequest,
        _environment: &ApiEnvironment,
        _base_url: &str,
        mut response: transom_api::ApiResponseSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.exec.assertion_results = evaluate_assertions(request, &response);
        for cookie in response.set_cookies.drain(..) {
            self.project.upsert_cookie(cookie);
        }
        self.load_cookies(window, cx);
        let (passed, total) = transom_api::assertion_summary(&self.exec.assertion_results);
        self.project.push_history(ApiHistoryEntry {
            request_id: request.id,
            request_name: request.display_name().to_string(),
            method: method_for_request(request),
            url: request.url.clone(),
            status: response.status,
            elapsed_millis: response.elapsed_millis,
            at: unix_now(),
            assertions_passed: passed,
            assertions_total: total,
            response_excerpt: response_excerpt(&response, 500),
        });
        self.ui.selected_history = Some(self.project.history.len().saturating_sub(1));
        self.editors.response_body.update(cx, |state, cx| {
            state.set_value(response.body.clone(), window, cx)
        });
        self.exec.response = Some(response);
        self.exec.error = None;
        cx.emit(super::ApiWorkspaceEvent::PersistProject {
            project: self.project.clone(),
            notice: None,
        });
    }

    fn clear_response_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editors
            .response_body
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
    }

    fn spawn_ws_events(
        &mut self,
        mut events: tokio::sync::mpsc::Receiver<ApiWebSocketEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.recv().await {
                let _ = this.update_in(cx, |this, window, cx| {
                    match event {
                        ApiWebSocketEvent::Opened => {
                            this.exec.ws_connected = true;
                            this.append_ws_log(
                                rust_i18n::t!("api.websocket.connected").to_string(),
                                window,
                                cx,
                            );
                        }
                        ApiWebSocketEvent::Message(text) => this.append_ws_log(text, window, cx),
                        ApiWebSocketEvent::Closed(reason) => {
                            this.exec.ws_connected = false;
                            this.exec.ws_handle = None;
                            this.append_ws_log(reason, window, cx);
                        }
                        ApiWebSocketEvent::Error(error) => {
                            this.exec.ws_connected = false;
                            this.exec.ws_handle = None;
                            this.append_ws_log(error, window, cx);
                        }
                    }
                    cx.notify();
                });
            }
        })
    }

    fn append_ws_log(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut log = self.editors.websocket_log.read(cx).value().to_string();
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&text);
        self.editors
            .websocket_log
            .update(cx, |state, cx| state.set_value(log, window, cx));
    }

    fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.exec.error = Some(message);
        self.exec.notice = None;
        cx.notify();
    }

    fn set_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.exec.notice = Some(message);
        self.exec.error = None;
        cx.notify();
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
