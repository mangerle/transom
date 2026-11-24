//! API 工作区请求草稿的读取与回写。

use super::workspace::ApiWorkspace;
use super::workspace_kv::{header_entry, header_from_entry, query_entry, query_from_entry};
use gpui_kit::*;
use transom_api::{ApiMethod, ApiProtocol};

impl ApiWorkspace {
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
        request.query = self
            .editors
            .query
            .read(cx)
            .entries(cx)
            .into_iter()
            .map(query_from_entry)
            .collect();
        request.headers = self
            .editors
            .headers
            .read(cx)
            .entries(cx)
            .into_iter()
            .map(header_from_entry)
            .collect();
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
        request.description = self.editors.docs.read(cx).value().to_string();
        let timeout = self.editors.timeout.read(cx).value().trim().to_string();
        request.timeout_seconds = if timeout.is_empty() {
            0
        } else {
            match timeout.parse::<u64>() {
                Ok(value) => value,
                Err(_) => {
                    self.error = Some(rust_i18n::t!("api.request.timeout_invalid").to_string());
                    return false;
                }
            }
        };
        self.project.upsert_request(request);
        self.capture_environment(cx);
        self.capture_cookies(cx);
        self.error = None;
        true
    }

    pub(super) fn load_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.selected_request().cloned();
        let query_entries = request
            .as_ref()
            .map(|request| request.query.iter().map(query_entry).collect::<Vec<_>>())
            .unwrap_or_default();
        let header_entries = request
            .as_ref()
            .map(|request| request.headers.iter().map(header_entry).collect::<Vec<_>>())
            .unwrap_or_default();
        let (
            name,
            url,
            body,
            method,
            protocol,
            graphql,
            auth,
            assertions,
            grpc_service,
            grpc_method,
            docs,
            timeout,
        ) = request.map_or_else(
            || {
                (
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
                    String::new(),
                    String::new(),
                )
            },
            |request| {
                let auth = request.auth_text();
                let assertions = request.assertions_text();
                (
                    request.display_name().to_string(),
                    request.url,
                    request.body,
                    request.method,
                    request.protocol,
                    request.graphql_variables,
                    auth,
                    assertions,
                    request.grpc_service,
                    request.grpc_method,
                    request.description,
                    if request.timeout_seconds == 0 {
                        String::new()
                    } else {
                        request.timeout_seconds.to_string()
                    },
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
        self.editors.query.update(cx, |editor, cx| {
            editor.set_entries(&query_entries, window, cx);
        });
        self.editors.headers.update(cx, |editor, cx| {
            editor.set_entries(&header_entries, window, cx);
        });
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
        self.editors
            .docs
            .update(cx, |state, cx| state.set_value(docs, window, cx));
        self.editors
            .timeout
            .update(cx, |state, cx| state.set_value(timeout, window, cx));
    }
}
