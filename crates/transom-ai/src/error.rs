//! `transom-ai` 领域错误定义。
//!
//! 设计原理（Why）：
//! - 底层与领域模块使用 `thiserror` 定义强类型中文错误枚举，不向调用方泄漏 `rig` 内部错误结构；
//! - 遵循“要么记录要么抛出”原则，此处只构造携带动态上下文的错误，不重复打印日志。

use thiserror::Error;

/// AI 助手运行与管理过程中的核心错误。
#[derive(Debug, Error)]
pub enum AiError {
    #[error("模型配置尚未就绪，缺少 {field}")]
    ConfigNotReady { field: &'static str },

    #[error("未找到指定模型配置，模型 ID: {model_id}")]
    ModelNotFound { model_id: String },

    #[error("请求大模型接口失败（协议: {protocol}，模型: {model}），原因: {reason}")]
    ModelCallFailed {
        protocol: String,
        model: String,
        reason: String,
    },

    #[error("已达到单次提问最大工具调用步数上限（{max_turns} 轮），运行自动中止")]
    MaxTurnsExceeded { max_turns: usize },

    #[error("操作已被用户取消")]
    Cancelled,

    #[error("操作超时（限时 {timeout_secs} 秒）: {operation}")]
    Timeout {
        operation: String,
        timeout_secs: u64,
    },

    #[error("访问路径越权或无效（{path}）: {reason}")]
    PathViolation { path: String, reason: String },

    #[error("会话或记忆存储读写失败（{target}），原因: {reason}")]
    StorageFailed { target: String, reason: String },

    #[error("图片处理失败，原因: {reason}")]
    ImageProcessFailed { reason: String },

    #[error("MCP 服务（{server}）通信失败，原因: {reason}")]
    McpFailed { server: String, reason: String },

    #[error("外部 ACP Agent（{agent}）不可用或通信失败，原因: {reason}")]
    AcpFailed { agent: String, reason: String },
}

/// 工具执行阶段返回给 Agent 与用户的错误。
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("工具参数解析失败（工具: {tool}），原因: {reason}")]
    InvalidArguments { tool: &'static str, reason: String },

    #[error("当前未绑定活动 SSH 会话，无法执行远端操作")]
    NoActiveSession,

    #[error("操作被安全策略拦截: {reason}")]
    PolicyDenied { reason: String },

    #[error("工具执行超时（限时 {timeout_secs} 秒）")]
    ExecutionTimeout { timeout_secs: u64 },

    #[error("工具执行被用户取消")]
    Cancelled,

    #[error("远端或本地执行失败（工具: {tool}），原因: {reason}")]
    ExecutionFailed { tool: &'static str, reason: String },
}
