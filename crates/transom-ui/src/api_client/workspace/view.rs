//! API 项目工作区根布局：接口树、请求编辑器和响应面板。

use super::request_view::render_request_editor;
use super::response_view::render_response;
use super::tree_view::render_tree;
use super::types::{ApiWorkspaceEditors, ExecutionState, WorkspaceSplits, WorkspaceUiState};
use super::{ApiSidebarState, ApiSidebarView, ApiWorkspace};
use crate::api_client::modals::{
    config::render_config_modal, new_request::render_new_request_modal,
};
use crate::divider::{h_resizable, v_resizable};
use crate::palette::Palette;
use crate::theme::tokens::rem_px;
use gpui_kit::base::v_flex;
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiEnvironmentId, ApiMethod, ApiProject, ApiProtocol, ApiRequest, ApiRequestId};

/// API 工作区渲染参数；所有跨实体数据由外壳在本帧读取后传入。
pub(crate) struct ApiWorkspaceParams<'a> {
    pub(crate) view: &'a Entity<ApiWorkspace>,
    pub(crate) api_sidebar: &'a Entity<ApiSidebarView>,
    pub(in crate::api_client) api_sidebar_state: &'a ApiSidebarState,
    pub(crate) project: &'a ApiProject,
    pub(crate) request: Option<&'a ApiRequest>,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) protocol: ApiProtocol,
    pub(crate) environment_id: Option<ApiEnvironmentId>,
    pub(in crate::api_client) ui: &'a WorkspaceUiState,
    pub(in crate::api_client) exec: &'a ExecutionState,
    pub(in crate::api_client) editors: &'a ApiWorkspaceEditors,
    pub(in crate::api_client) splits: &'a WorkspaceSplits,
    pub(crate) rem: Pixels,
}

impl ApiWorkspace {
    /// 组装本帧渲染所需的只读参数对象，避免外层窗格逐字段解构拷贝。
    pub(crate) fn render_params<'a>(
        &'a self,
        view: &'a Entity<Self>,
        api_sidebar: &'a Entity<ApiSidebarView>,
        api_sidebar_state: &'a ApiSidebarState,
        rem: Pixels,
    ) -> ApiWorkspaceParams<'a> {
        ApiWorkspaceParams {
            view,
            api_sidebar,
            api_sidebar_state,
            project: &self.project,
            request: self.selected_request(),
            selected_request: self.selected_request,
            method: self.method,
            protocol: self.protocol,
            environment_id: self.environment_id,
            ui: &self.ui,
            exec: &self.exec,
            editors: &self.editors,
            splits: &self.splits,
            rem,
        }
    }
}

/// 渲染 API 项目工作区。
pub(crate) fn render_api_workspace(params: ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let tree = render_tree(&params, p);
    let editor = render_request_editor(&params, p);
    let response = render_response(&params, p);
    let editor_and_response = v_resizable("api-request-response-split")
        .with_state(&params.splits.request_response)
        .child(
            resizable_panel()
                .size(rem_px(28.0, params.rem))
                .child(editor),
        )
        .child(resizable_panel().child(response));
    let body = h_resizable("api-tree-editor-split")
        .with_state(&params.splits.tree_editor)
        .child(resizable_panel().size(rem_px(16.0, params.rem)).child(tree))
        .child(resizable_panel().child(editor_and_response));

    v_flex()
        .relative()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(body)
        .when_some(params.ui.config_modal, |root, section| {
            root.child(render_config_modal(&params, section, p))
        })
        .when(params.ui.new_request_modal_open, |root| {
            root.child(render_new_request_modal(&params, p))
        })
        .into_any_element()
}
