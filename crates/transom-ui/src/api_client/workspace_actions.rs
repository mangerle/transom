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
        if self.protocol == ApiProtocol::WebSocket {
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
        let resolved = request.resolved(&self.project.base_url, &environment);
        let cookies = self.project.cookies.clone();
        self.sending = true;
        self.response = None;
        self.assertion_results.clear();
        self.clear_response_editor(window, cx);
        self.error = None;
        cx.notify();

        let http = self.services.http.clone();
        let project_base = self.project.base_url.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let mut exchange = http.start(resolved.clone(), environment.clone(), cookies);
            let result = exchange.recv().await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = false;
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
                        this.response = None;
                        this.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        });
        self.send_task = Some(task);
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
        let url = request.resolved_url(&self.project.base_url, &environment);
        let headers = request.headers.clone();
        let service = self.services.websocket.clone();
        let start = service.start(url, headers, environment, request.auth.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = start.into_handle().await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(mut handle) => {
                    let events = handle.take_events();
                    this.ws_handle = Some(handle);
                    this.ws_connected = true;
                    this.append_ws_log(
                        rust_i18n::t!("api.websocket.connected").to_string(),
                        window,
                        cx,
                    );
                    this.ws_event_task = Some(this.spawn_ws_events(events, window, cx));
                }
                Err(error) => this.set_error(error, cx),
            });
        });
        self.ws_task = Some(task);
    }

    /// 发送一条 WebSocket 文本消息。
    pub(crate) fn send_websocket_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editors.websocket_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some(handle) = &self.ws_handle else {
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
        if let Some(handle) = self.ws_handle.take() {
            handle.close();
        }
        self.ws_connected = false;
        self.ws_event_task = None;
        self.set_notice(rust_i18n::t!("api.websocket.closed").to_string(), cx);
    }

    /// 生成代码并复制到剪贴板。
    pub(crate) fn copy_code(&mut self, target: CodeTarget, cx: &mut Context<Self>) {
        if !self.capture_request(cx) {
            return;
        }
        let Some(request) = self.selected_request().cloned() else {
            return;
        };
        let request = request.resolved(&self.project.base_url, &self.environment());
        let code = generate_code(target, &request);
        cx.write_to_clipboard(ClipboardItem::new_string(code));
        self.set_notice(rust_i18n::t!("api.code.copied").to_string(), cx);
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
        self.assertion_results = evaluate_assertions(request, &response);
        for cookie in response.set_cookies.drain(..) {
            self.project.upsert_cookie(cookie);
        }
        let (passed, total) = transom_api::assertion_summary(&self.assertion_results);
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
        self.refresh_history(window, cx);
        self.editors.response_body.update(cx, |state, cx| {
            state.set_value(response.body.clone(), window, cx)
        });
        self.response = Some(response);
        self.error = None;
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
                            this.ws_connected = true;
                            this.append_ws_log(
                                rust_i18n::t!("api.websocket.connected").to_string(),
                                window,
                                cx,
                            );
                        }
                        ApiWebSocketEvent::Message(text) => this.append_ws_log(text, window, cx),
                        ApiWebSocketEvent::Closed(reason) => {
                            this.ws_connected = false;
                            this.append_ws_log(reason, window, cx);
                        }
                        ApiWebSocketEvent::Error(error) => {
                            this.ws_connected = false;
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
        self.error = Some(message);
        self.notice = None;
        cx.notify();
    }

    fn set_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = Some(message);
        self.error = None;
        cx.notify();
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
