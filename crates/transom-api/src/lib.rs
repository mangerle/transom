//! Transom 桌面端 API 客户端领域。
//!
//! 设计原理（Why）：API 项目、接口树和请求定义属于稳定的领域数据；HTTP 执行
//! 与本地 JSON 存储分别由本 crate 的服务和 `transom-store` 实现承载。这样 UI
//! 只依赖一个清晰的 API 能力边界，不需要直接引入 CyberAPI 的任何 crate，也不会
//! 让旧项目的账号、团队、邀请或服务端同步模型渗入 Transom。
//!
//! 代价与局限：当前版本提供桌面端本地项目、HTTP/GraphQL/原始 gRPC/WebSocket
//! 请求闭环；OpenAPI 服务端工程生成、完整 GraphQL schema 补全和 gRPC proto
//! 代码生成仍不在本阶段。

mod assertions;
mod codegen;
mod http;
mod http_body;
mod http_support;
mod id;
mod import_export;
mod model;
mod request;
mod signing;
mod store;
mod websocket;

pub use assertions::{assertion_summary, evaluate_assertions};
pub use codegen::{CodeTarget, generate_code, response_excerpt};
pub use http::{
    ApiError, ApiExchange, ApiHttpService, ApiResponseSnapshot, MAX_RESPONSE_BYTES,
    MAX_UPLOAD_BYTES,
};
pub use id::{ApiEnvironmentId, ApiFolderId, ApiProjectId, ApiRequestId};
pub use import_export::{
    ApiExportFormat, ApiImportFormat, detect_import_format, export_project, import_projects,
    import_projects_auto,
};
pub use model::{
    API_SCHEMA_VERSION, ApiCookie, ApiEnvironment, ApiEnvironmentResolver, ApiFolder,
    ApiGlobalParameter, ApiGlobalParameterTarget, ApiHistoryEntry, ApiProject, ApiRegistry,
    ApiVariable, MAX_IMPORT_BYTES, cookies_text, parse_cookies_text,
};
pub use request::{
    ApiAssertion, ApiAssertionResult, ApiAuth, ApiBodyType, ApiExecutionPlan, ApiFormField,
    ApiFormFieldKind, ApiHeader, ApiMethod, ApiProtocol, ApiQueryParam, ApiRawLanguage, ApiRequest,
    method_for_request,
};
pub use store::{ApiProjectStore, parse_project_import};
pub use websocket::{
    ApiWebSocketError, ApiWebSocketEvent, ApiWebSocketHandle, ApiWebSocketService,
    ApiWebSocketStart,
};
