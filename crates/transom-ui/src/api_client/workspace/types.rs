//! API 工作区公开类型与编辑器集合。

use gpui_kit::component::input::{EditorState, InputState, TextareaState};
use gpui_kit::*;
use transom_api::{
    ApiBodyType, ApiEnvironmentId, ApiHttpService, ApiProject, ApiRawLanguage, ApiWebSocketService,
};

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
    GlobalParameters,
    GlobalVariables,
    Cookies,
    Code,
}

/// 环境变量与全局配置弹窗的左侧导航项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiConfigModalSection {
    Cookies,
    GlobalParameters,
    GlobalVariables,
    Environment(ApiEnvironmentId),
}

/// 全局参数编辑页签。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ApiGlobalParameterTab {
    #[default]
    Query,
    Header,
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

/// HTTP 请求体的顶层编辑模式；raw 的具体语言单独选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiBodyMode {
    None,
    FormData,
    UrlEncoded,
    Binary,
    Raw,
}

impl ApiBodyMode {
    pub(crate) const ALL: [Self; 5] = [
        Self::None,
        Self::FormData,
        Self::UrlEncoded,
        Self::Binary,
        Self::Raw,
    ];

    pub(crate) fn from_body_type(body_type: ApiBodyType) -> Self {
        match body_type {
            ApiBodyType::None => Self::None,
            ApiBodyType::Multipart => Self::FormData,
            ApiBodyType::FormUrlEncoded => Self::UrlEncoded,
            ApiBodyType::Binary => Self::Binary,
            ApiBodyType::Json
            | ApiBodyType::Text
            | ApiBodyType::Xml
            | ApiBodyType::JavaScript
            | ApiBodyType::Html => Self::Raw,
        }
    }

    pub(crate) fn body_type(self, raw_language: ApiRawLanguage) -> ApiBodyType {
        match self {
            Self::None => ApiBodyType::None,
            Self::FormData => ApiBodyType::Multipart,
            Self::UrlEncoded => ApiBodyType::FormUrlEncoded,
            Self::Binary => ApiBodyType::Binary,
            Self::Raw => raw_language.body_type(),
        }
    }
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
    /// 工作区结构已变化，只同步侧栏和标签内存状态，不立即落盘。
    SyncProject { project: ApiProject },
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
    pub(crate) query: Entity<crate::api_client::editors::kv::KvEditor>,
    pub(crate) headers: Entity<crate::api_client::editors::kv::KvEditor>,
    pub(crate) request_body: Entity<EditorState>,
    pub(crate) grpc_service: Entity<InputState>,
    pub(crate) grpc_method: Entity<InputState>,
    pub(crate) graphql_variables: Entity<TextareaState>,
    pub(crate) auth: Entity<crate::api_client::editors::auth::AuthEditor>,
    pub(crate) assertions: Entity<crate::api_client::editors::assertion::AssertionEditor>,
    pub(crate) timeout: Entity<InputState>,
    pub(crate) docs: Entity<TextareaState>,
    pub(crate) environment: Entity<crate::api_client::editors::environment::EnvironmentEditor>,
    pub(crate) global_variables: Entity<crate::api_client::editors::environment::EnvironmentEditor>,
    pub(crate) global_query: Entity<crate::api_client::editors::kv::KvEditor>,
    pub(crate) global_headers: Entity<crate::api_client::editors::kv::KvEditor>,
    pub(crate) environment_name: Entity<InputState>,
    pub(crate) cookies: Entity<crate::api_client::editors::cookie::CookieEditor>,
    pub(crate) form: Entity<crate::api_client::editors::form::FormEditor>,
    pub(crate) websocket_input: Entity<TextareaState>,
    pub(crate) websocket_log: Entity<TextareaState>,
    pub(crate) response_body: Entity<TextareaState>,
    pub(crate) code_preview: Entity<TextareaState>,
    pub(crate) save_destination_search: Entity<InputState>,
}
