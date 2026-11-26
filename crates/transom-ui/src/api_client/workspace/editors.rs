//! API 工作区文本编辑器实体构造。
//!
//! 将实体创建从工作区状态文件中分离，保证状态流转与编辑器初始化各自内聚，
//! 同时避免单个文件随高级配置项增长突破 500 行上限。

use super::ApiWorkspace;
use super::ApiWorkspaceEditors;
use crate::api_client::editors::assertion::AssertionEditor;
use crate::api_client::editors::auth::AuthEditor;
use crate::api_client::editors::cookie::CookieEditor;
use crate::api_client::editors::environment::EnvironmentEditor;
use crate::api_client::editors::form::FormEditor;
use crate::api_client::editors::kv::{KvEditor, KvEditorKind};
use gpui_kit::component::input::{EditorState, InputState, TextareaState};
use gpui_kit::*;

/// 为单个 API 项目工作区创建全部文本编辑器实体。
pub(super) fn new_editors(
    window: &mut Window,
    cx: &mut Context<ApiWorkspace>,
) -> ApiWorkspaceEditors {
    let placeholder = |window: &mut Window, cx: &mut Context<ApiWorkspace>, key: &'static str| {
        cx.new(|cx| TextareaState::new(window, cx).placeholder(rust_i18n::t!(key).to_string()))
    };
    ApiWorkspaceEditors {
        request_name: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.name_placeholder").to_string())
        }),
        request_url: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.url_placeholder").to_string())
        }),
        query: cx.new(|cx| KvEditor::new(KvEditorKind::Query, window, cx)),
        headers: cx.new(|cx| KvEditor::new(KvEditorKind::Header, window, cx)),
        request_body: cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(true)
        }),
        grpc_service: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.grpc_service_placeholder").to_string())
        }),
        grpc_method: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.grpc_method_placeholder").to_string())
        }),
        graphql_variables: placeholder(window, cx, "api.request.graphql_variables_placeholder"),
        auth: cx.new(|cx| AuthEditor::new(window, cx)),
        assertions: cx.new(|cx| AssertionEditor::new(window, cx)),
        timeout: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.timeout_hint").to_string())
        }),
        docs: placeholder(window, cx, "api.request.docs_placeholder"),
        environment: cx.new(|cx| EnvironmentEditor::new(window, cx)),
        global_variables: cx.new(|cx| EnvironmentEditor::new(window, cx)),
        global_query: cx.new(|cx| KvEditor::new(KvEditorKind::Query, window, cx)),
        global_headers: cx.new(|cx| KvEditor::new(KvEditorKind::Header, window, cx)),
        environment_name: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.environment_name").to_string())
        }),
        cookies: cx.new(|cx| CookieEditor::new(window, cx)),
        form: cx.new(|cx| FormEditor::new(window, cx)),
        websocket_input: placeholder(window, cx, "api.request.websocket_input_placeholder"),
        response_body: cx.new(|cx| TextareaState::new(window, cx)),
        code_preview: cx.new(|cx| TextareaState::new(window, cx)),
        save_destination_search: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.save_destination_placeholder").to_string())
        }),
    }
}
