//! API 工作区公开类型与编辑器集合。

use super::folder::FolderEditor;
use gpui_kit::component::input::{EditorState, InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::VecDeque;
use transom_api::{
    ApiAssertionResult, ApiBodyType, ApiEnvironmentId, ApiFolderId, ApiHttpService, ApiProject,
    ApiRawLanguage, ApiRequestId, ApiResponseSnapshot, ApiWebSocketHandle, ApiWebSocketService,
    CodeTarget,
};

/// API 工作区需要的外壳服务集合。
#[derive(Clone)]
pub(crate) struct ApiWorkspaceServices {
    pub(crate) http: ApiHttpService,
    pub(crate) websocket: ApiWebSocketService,
}

/// 项目级辅助面板；目前承载请求代码预览。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiAuxiliarySection {
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

/// 工作区交互与界面视图状态（页签、弹窗、辅助面板、目录编辑等）。
pub(in crate::api_client) struct WorkspaceUiState {
    pub(in crate::api_client) request_tab: ApiRequestTab,
    pub(in crate::api_client) response_tab: ApiResponseTab,
    pub(in crate::api_client) selected_history: Option<usize>,
    pub(in crate::api_client) save_destination_open: bool,
    pub(in crate::api_client) auxiliary_section: Option<ApiAuxiliarySection>,
    pub(in crate::api_client) config_modal: Option<ApiConfigModalSection>,
    pub(in crate::api_client) new_request_modal_open: bool,
    pub(in crate::api_client) global_parameter_tab: ApiGlobalParameterTab,
    pub(in crate::api_client) codegen_target: CodeTarget,
    pub(in crate::api_client) folder_editor: Option<FolderEditor>,
    pub(in crate::api_client) pending_folder_delete: Option<ApiFolderId>,
    pub(in crate::api_client) collapsed_folders: std::collections::HashSet<ApiFolderId>,
}

impl Default for WorkspaceUiState {
    fn default() -> Self {
        Self {
            request_tab: ApiRequestTab::Query,
            response_tab: ApiResponseTab::Body,
            selected_history: None,
            save_destination_open: false,
            auxiliary_section: None,
            config_modal: None,
            new_request_modal_open: false,
            global_parameter_tab: ApiGlobalParameterTab::Query,
            codegen_target: CodeTarget::Curl,
            folder_editor: None,
            pending_folder_delete: None,
            collapsed_folders: std::collections::HashSet::new(),
        }
    }
}

/// WebSocket 日志有界环形缓冲区上限。
const MAX_WS_LOG_LINES: usize = 1_000;
const MAX_WS_LOG_BYTES: usize = 256 * 1024;

/// 有界 WebSocket 消息日志缓冲区，避免全量回读与无上限内存增长。
#[derive(Default)]
pub(in crate::api_client) struct WsLogBuffer {
    lines: VecDeque<String>,
    total_bytes: usize,
}

impl WsLogBuffer {
    /// 批量追加日志，只在批次结束时构造一次完整文本。
    pub(in crate::api_client) fn push_many(
        &mut self,
        new_lines: impl IntoIterator<Item = String>,
    ) -> String {
        for line in new_lines {
            let line = truncate_log_tail(line, MAX_WS_LOG_BYTES);
            self.total_bytes = self.total_bytes.saturating_add(line.len());
            self.lines.push_back(line);
        }
        while (self.lines.len() > MAX_WS_LOG_LINES || self.total_bytes > MAX_WS_LOG_BYTES)
            && self.lines.len() > 1
        {
            if let Some(oldest) = self.lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(oldest.len());
            }
        }
        let mut out = String::with_capacity(self.total_bytes + self.lines.len());
        for (index, item) in self.lines.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(item);
        }
        out
    }
}

fn truncate_log_tail(mut line: String, limit: usize) -> String {
    if line.len() <= limit {
        return line;
    }
    let mut start = line.len().saturating_sub(limit);
    while !line.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    line.drain(..start);
    line
}

/// 请求执行、WebSocket 连接、响应快照与用户提示状态。
#[derive(Default)]
pub(in crate::api_client) struct ExecutionState {
    pub(in crate::api_client) response: Option<ApiResponseSnapshot>,
    pub(in crate::api_client) assertion_results: Vec<ApiAssertionResult>,
    pub(in crate::api_client) send_generation: u64,
    pub(in crate::api_client) sending_request: Option<ApiRequestId>,
    pub(in crate::api_client) ws_connected: bool,
    pub(in crate::api_client) notice: Option<String>,
    pub(in crate::api_client) error: Option<String>,
    pub(in crate::api_client) send_task: Option<Task<()>>,
    pub(in crate::api_client) ws_task: Option<Task<()>>,
    pub(in crate::api_client) ws_event_task: Option<Task<()>>,
    pub(in crate::api_client) ws_handle: Option<ApiWebSocketHandle>,
    pub(in crate::api_client) ws_log_buffer: WsLogBuffer,
    pub(in crate::api_client) binary_file_task: Option<Task<()>>,
}

impl ExecutionState {
    pub(in crate::api_client) fn is_sending(&self) -> bool {
        self.sending_request.is_some()
    }

    pub(in crate::api_client) fn begin_send(&mut self, request_id: ApiRequestId) -> u64 {
        self.send_generation = self.send_generation.saturating_add(1);
        self.sending_request = Some(request_id);
        self.send_generation
    }

    pub(in crate::api_client) fn finish_send(&mut self, generation: u64) -> bool {
        if self.send_generation != generation {
            return false;
        }
        self.sending_request = None;
        true
    }
}

/// 工作区可拖拽分栏状态。
pub(in crate::api_client) struct WorkspaceSplits {
    pub(in crate::api_client) tree_editor: Entity<ResizableState>,
    pub(in crate::api_client) request_response: Entity<ResizableState>,
}

impl WorkspaceSplits {
    /// 初始化工作区左右树栏与上下编辑/响应分栏。
    pub(in crate::api_client) fn new<T: 'static>(cx: &mut Context<T>) -> Self {
        Self {
            tree_editor: cx.new(|_| ResizableState::default()),
            request_response: cx.new(|_| ResizableState::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutionState, MAX_WS_LOG_BYTES, MAX_WS_LOG_LINES, WsLogBuffer};
    use transom_api::ApiRequestId;

    #[test]
    fn ws_log_buffer_evicts_oldest_when_exceeding_limits() {
        let mut buffer = WsLogBuffer::default();
        for index in 0..(MAX_WS_LOG_LINES + 5) {
            let _ = buffer.push_many([format!("line-{index}")]);
        }
        assert_eq!(buffer.lines.len(), MAX_WS_LOG_LINES);
        assert_eq!(buffer.lines.front().map(String::as_str), Some("line-5"));
        let oversized = "x".repeat(MAX_WS_LOG_BYTES.saturating_add(100));
        let text = buffer.push_many([oversized]);
        assert_eq!(buffer.lines.len(), 1);
        assert!(text.len() <= MAX_WS_LOG_BYTES);
    }

    #[test]
    fn stale_send_completion_does_not_clear_newer_send_state() {
        let mut state = ExecutionState::default();
        let first = state.begin_send(ApiRequestId::new());
        let second = state.begin_send(ApiRequestId::new());
        assert!(!state.finish_send(first));
        assert!(state.is_sending());
        assert!(state.finish_send(second));
        assert!(!state.is_sending());
    }
}
