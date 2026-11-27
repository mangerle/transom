//! MCP（Model Context Protocol）服务器配置与统一风险代理契约。
//!
//! 设计原理（Why）：
//! - 将外部 MCP 工具（如 K8s、Prometheus、内部 CMDB）纳入 Transom 统一工具注册表；
//! - 未显式列入 `trusted_readonly_tools` 的外部工具默认按 `ToolRisk::Write` 送审，
//!   避免外部 MCP 服务绕过审批门直接产生副作用。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::policy::ToolRisk;

/// MCP 传输方式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    /// 本地子进程（stdio JSON-RPC）。
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// 远端 HTTP / Streamable SSE 端点。
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// 单个 MCP 服务器配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub transport: McpTransport,
    /// 用户显式信任为只读的工具名称列表（其余一律按 Write 送审）。
    #[serde(default)]
    pub trusted_readonly_tools: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// 外部 MCP 工具在 Transom 风险体系下的代理描述。
#[derive(Clone, Debug)]
pub struct McpToolDescriptor {
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub risk: ToolRisk,
}

impl McpServerConfig {
    /// 判定该服务器上指定工具的风险级别。
    pub fn classify_tool_risk(&self, tool_name: &str) -> ToolRisk {
        if self
            .trusted_readonly_tools
            .iter()
            .any(|name| name == tool_name)
        {
            ToolRisk::SafeRead
        } else {
            ToolRisk::Write
        }
    }
}
