//! 软件全局操作抽象桥接 Trait 与强类型领域枚举。
//!
//! 设计原理（Why）：
//! - `transom-ai` 仅定义纯数据契约，不依赖 GPUI、窗口实体或控制平面状态；
//! - 通过 [`AppBridgeDomain`] 强类型枚举取代无类型字符串路由，在编译期保证
//!   内置 AI 工具与外部 ACP（MCP Server）共用同一套领域契约与穷举分发。

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ToolError;

/// 软件操作桥接器支持的强类型领域枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppBridgeDomain {
    SessionCatalog,
    WorkspaceControl,
    HostMonitor,
    PortForwardOps,
    SftpTransfer,
    ContainerWorkspace,
    GitWorkspace,
    ApiClient,
    AppSettingsOps,
    AiMemorySkills,
    RemoteExec,
    RemoteFs,
    TransferFile,
}

impl AppBridgeDomain {
    /// 面向内置 AI 的软件操作领域清单，不包含远端执行/文件/传输路由。
    pub const SOFTWARE_DOMAINS: [Self; 10] = [
        Self::SessionCatalog,
        Self::WorkspaceControl,
        Self::HostMonitor,
        Self::PortForwardOps,
        Self::SftpTransfer,
        Self::ContainerWorkspace,
        Self::GitWorkspace,
        Self::ApiClient,
        Self::AppSettingsOps,
        Self::AiMemorySkills,
    ];

    /// 判断名称是否属于软件操作领域。
    pub fn is_software_tool_name(raw: &str) -> bool {
        Self::SOFTWARE_DOMAINS
            .iter()
            .any(|domain| domain.as_str() == raw)
    }

    /// 返回与工具名称对齐的标准蛇形标识。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionCatalog => "session_catalog",
            Self::WorkspaceControl => "workspace_control",
            Self::HostMonitor => "host_monitor",
            Self::PortForwardOps => "port_forward_ops",
            Self::SftpTransfer => "sftp_transfer",
            Self::ContainerWorkspace => "container_workspace",
            Self::GitWorkspace => "git_workspace",
            Self::ApiClient => "api_client",
            Self::AppSettingsOps => "app_settings_ops",
            Self::AiMemorySkills => "ai_memory_skills",
            Self::RemoteExec => "remote_exec",
            Self::RemoteFs => "remote_fs",
            Self::TransferFile => "transfer_file",
        }
    }

    /// 从工具名称解析强类型领域变体。
    ///
    /// # Errors
    /// 当传入未知工具领域名称时返回 [`ToolError::InvalidArguments`]。
    pub fn parse(raw: &str) -> Result<Self, ToolError> {
        match raw {
            "session_catalog" => Ok(Self::SessionCatalog),
            "workspace_control" => Ok(Self::WorkspaceControl),
            "host_monitor" => Ok(Self::HostMonitor),
            "port_forward_ops" => Ok(Self::PortForwardOps),
            "sftp_transfer" => Ok(Self::SftpTransfer),
            "container_workspace" => Ok(Self::ContainerWorkspace),
            "git_workspace" => Ok(Self::GitWorkspace),
            "api_client" => Ok(Self::ApiClient),
            "app_settings_ops" => Ok(Self::AppSettingsOps),
            "ai_memory_skills" => Ok(Self::AiMemorySkills),
            "remote_exec" => Ok(Self::RemoteExec),
            "remote_fs" => Ok(Self::RemoteFs),
            "transfer_file" => Ok(Self::TransferFile),
            other => Err(ToolError::InvalidArguments {
                tool: "app_bridge",
                reason: format!("未知软件操作领域: {other}"),
            }),
        }
    }
}

impl fmt::Display for AppBridgeDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 软件级操作执行器抽象（由控制平面与 UI 联合实现）。
#[async_trait]
pub trait AppActionBridge: Send + Sync {
    /// 执行指定领域与动作的软件操作，返回面向模型的可读文本或 JSON 摘要。
    async fn invoke(
        &self,
        domain: AppBridgeDomain,
        action: &str,
        args: Value,
    ) -> Result<String, ToolError>;
}
