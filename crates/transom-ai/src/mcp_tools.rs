//! 反向 MCP Server 对外暴露的工具 Schema 清单。
//!
//! 设计原理（Why）：
//! - 直接复用 [`crate::tools::export_mcp_tool_schemas`] 从 `rig` 工具实现导出 Schema，
//!   仅补充 MCP 独有的 `list_sessions` 会话枚举工具，彻底消除双套 Schema 漂移。

use serde_json::{Value, json};

/// 返回全部向外部 ACP / MCP 客户端暴露的工具定义。
pub(crate) fn tool_definitions() -> Vec<Value> {
    let mut tools = Vec::with_capacity(20);
    tools.push(json!({
        "name": "list_sessions",
        "description": "列出当前 Transom 中已连接的 SSH 会话（脱敏元数据，不含密钥与密码）。",
        "inputSchema": { "type": "object", "properties": {} }
    }));
    tools.extend(crate::tools::export_mcp_tool_schemas());
    tools
}
