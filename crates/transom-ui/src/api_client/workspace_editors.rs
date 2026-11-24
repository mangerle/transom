//! API 工作区文本编辑器实体构造。
//!
//! 将实体创建从工作区状态文件中分离，保证状态流转与编辑器初始化各自内聚，
//! 同时避免单个文件随高级配置项增长突破 500 行上限。

use super::workspace::ApiWorkspace;
use super::workspace::ApiWorkspaceEditors;
use gpui_kit::component::input::{InputState, TextareaState};
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
        request_headers: placeholder(window, cx, "api.request.headers_placeholder"),
        request_body: placeholder(window, cx, "api.request.body_placeholder"),
        grpc_service: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.grpc_service_placeholder").to_string())
        }),
        grpc_method: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.grpc_method_placeholder").to_string())
        }),
        graphql_variables: placeholder(window, cx, "api.request.graphql_variables_placeholder"),
        auth: placeholder(window, cx, "api.request.auth_placeholder"),
        assertions: placeholder(window, cx, "api.request.assertions_placeholder"),
        environment: placeholder(window, cx, "api.request.environment_placeholder"),
        cookies: placeholder(window, cx, "api.request.cookies_placeholder"),
        mock: placeholder(window, cx, "api.request.mock_placeholder"),
        history: cx.new(|cx| TextareaState::new(window, cx)),
        websocket_input: placeholder(window, cx, "api.request.websocket_input_placeholder"),
        websocket_log: cx.new(|cx| TextareaState::new(window, cx)),
        response_headers: placeholder(window, cx, "api.response.headers_placeholder"),
        response_body: cx.new(|cx| TextareaState::new(window, cx)),
    }
}
