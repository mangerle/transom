//! API 工作区公开类型与编辑器集合。

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use transom_api::{ApiHttpService, ApiProject, ApiWebSocketService};

/// API 工作区需要的外壳服务集合。
#[derive(Clone)]
pub(crate) struct ApiWorkspaceServices {
    pub(crate) http: ApiHttpService,
    pub(crate) websocket: ApiWebSocketService,
}

/// 项目级配置面板；只承载不随单个请求切换的配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiAuxiliarySection {
    Environment,
    Cookies,
}

/// 请求编辑器的信息架构页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiRequestTab {
    Query,
    Body,
    Headers,
    Auth,
    Assertions,
    Settings,
    Docs,
}

/// 响应面板的信息架构页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiResponseTab {
    Body,
    Headers,
    Cookies,
    Assertions,
    History,
}

/// API 工作区需要外壳处理的窗口级意图。
#[derive(Clone)]
pub(crate) enum ApiWorkspaceEvent {
    /// 项目快照已变化，请求由外壳更新登记表并异步落盘。
    PersistProject {
        project: ApiProject,
        notice: Option<String>,
    },
}

/// 所有可编辑文本实体集中在一个参数对象中，避免渲染函数反查工作区实体。
pub(crate) struct ApiWorkspaceEditors {
    pub(crate) request_name: Entity<InputState>,
    pub(crate) request_url: Entity<InputState>,
    pub(crate) query: Entity<super::kv_editor::KvEditor>,
    pub(crate) headers: Entity<super::kv_editor::KvEditor>,
    pub(crate) request_body: Entity<TextareaState>,
    pub(crate) grpc_service: Entity<InputState>,
    pub(crate) grpc_method: Entity<InputState>,
    pub(crate) graphql_variables: Entity<TextareaState>,
    pub(crate) auth: Entity<TextareaState>,
    pub(crate) assertions: Entity<TextareaState>,
    pub(crate) timeout: Entity<InputState>,
    pub(crate) docs: Entity<TextareaState>,
    pub(crate) environment: Entity<TextareaState>,
    pub(crate) cookies: Entity<TextareaState>,
    pub(crate) history: Entity<TextareaState>,
    pub(crate) websocket_input: Entity<TextareaState>,
    pub(crate) websocket_log: Entity<TextareaState>,
    pub(crate) response_headers: Entity<TextareaState>,
    pub(crate) response_body: Entity<TextareaState>,
}
