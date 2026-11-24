//! API 项目工作区根布局：接口树、请求编辑器和响应面板。

use super::workspace_request_view::render_request_editor;
use super::workspace_response_view::render_response;
use super::{ApiAuxiliarySection, ApiRequestTab, ApiResponseTab, ApiWorkspace};
use crate::divider::{h_resizable, v_resizable};
use crate::palette::Palette;
use crate::theme::tokens::rem_px;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::*;
use transom_api::{
    ApiAssertionResult, ApiEnvironmentId, ApiMethod, ApiProject, ApiProtocol, ApiRequest,
    ApiRequestId, ApiResponseSnapshot,
};

/// API 工作区渲染参数；所有跨实体数据由外壳在本帧读取后传入。
pub(crate) struct ApiWorkspaceParams<'a> {
    pub(crate) view: &'a Entity<ApiWorkspace>,
    pub(crate) project: &'a ApiProject,
    pub(crate) request: Option<&'a ApiRequest>,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) protocol: ApiProtocol,
    pub(crate) environment_id: Option<ApiEnvironmentId>,
    pub(crate) request_tab: ApiRequestTab,
    pub(crate) editor_ratio: f32,
    pub(crate) response_tab: ApiResponseTab,
    pub(crate) auxiliary_section: Option<ApiAuxiliarySection>,
    pub(crate) ws_connected: bool,
    pub(crate) notice: Option<&'a str>,
    pub(crate) response: Option<&'a ApiResponseSnapshot>,
    pub(crate) sending: bool,
    pub(crate) error: Option<&'a str>,
    pub(crate) request_url: &'a Entity<InputState>,
    pub(crate) query_editor: &'a Entity<super::kv_editor::KvEditor>,
    pub(crate) header_editor: &'a Entity<super::kv_editor::KvEditor>,
    pub(crate) request_body: &'a Entity<TextareaState>,
    pub(crate) form_editor: &'a Entity<super::form_editor::FormEditor>,
    pub(crate) grpc_service: &'a Entity<InputState>,
    pub(crate) grpc_method: &'a Entity<InputState>,
    pub(crate) graphql_variables: &'a Entity<TextareaState>,
    pub(crate) auth: &'a Entity<super::auth_editor::AuthEditor>,
    pub(crate) assertions: &'a Entity<super::assertion_editor::AssertionEditor>,
    pub(crate) timeout: &'a Entity<InputState>,
    pub(crate) docs: &'a Entity<TextareaState>,
    pub(crate) environment_editor: &'a Entity<super::environment_editor::EnvironmentEditor>,
    pub(crate) cookies_editor: &'a Entity<super::cookie_editor::CookieEditor>,
    pub(crate) websocket_input: &'a Entity<TextareaState>,
    pub(crate) websocket_log: &'a Entity<TextareaState>,
    pub(crate) assertion_results: &'a [ApiAssertionResult],
    pub(crate) response_body: &'a Entity<TextareaState>,
    pub(crate) tree_editor_split: &'a Entity<ResizableState>,
    pub(crate) request_response_split: &'a Entity<ResizableState>,
    pub(crate) rem: Pixels,
}

/// 渲染 API 项目工作区。
pub(crate) fn render_api_workspace(params: ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let tree = super::workspace_tree_view::render_tree(&params, p);
    let editor = render_request_editor(&params, p);
    let response = render_response(&params, p);
    let editor_and_response = v_resizable("api-request-response-split")
        .with_state(params.request_response_split)
        .child(
            resizable_panel()
                .size(rem_px(28.0, params.rem))
                .child(editor),
        )
        .child(resizable_panel().child(response));
    let body = h_resizable("api-tree-editor-split")
        .with_state(params.tree_editor_split)
        .child(resizable_panel().size(rem_px(16.0, params.rem)).child(tree))
        .child(resizable_panel().child(editor_and_response));

    v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(body)
        .into_any_element()
}
