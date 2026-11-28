//! 工具目标解析契约与策略目标快照。
//!
//! 策略门必须在审批前知道实际目标主机的标识与保护级别，不能等工具开始执行后才解析
//! `target`。本模块把目标解析抽象注入运行时上下文，让内置 Agent、MCP 与子 Agent
//! 共用同一套策略目标语义。

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;
use crate::policy::HostProtection;

/// 已完成策略解析的工具目标。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedToolTarget {
    /// 稳定主机标识；本机或未绑定主机的全局上下文为 `None`。
    pub host_id: Option<String>,
    /// 审批卡片与工具卡片使用的规范展示名称。
    pub label: String,
    /// 实际目标主机的保护级别。
    pub protection: HostProtection,
}

impl ResolvedToolTarget {
    /// 构造一个已解析的工具目标。
    pub fn new(
        host_id: Option<String>,
        label: impl Into<String>,
        protection: HostProtection,
    ) -> Self {
        Self {
            host_id,
            label: label.into(),
            protection,
        }
    }
}

/// 将工具参数中的目标标识解析为策略目标快照。
///
/// 解析器只负责读取保存的会话或当前已连接会话的元数据，不应在这里建立新连接；
/// 建立连接仍由工具执行层负责，避免审批前产生未授权的外部副作用。
#[async_trait]
pub trait ToolTargetResolver: Send + Sync {
    /// 解析远端目标名称、会话 ID 或主机地址。
    ///
    /// # Errors
    /// 目标不存在、匹配到多个主机或解析后端不可用时返回 [`ToolError`]。
    async fn resolve_remote_target(&self, target: &str) -> Result<ResolvedToolTarget, ToolError>;
}

/// 从工具参数提取需要参与策略解析的目标字段。
///
/// 只有真正具有远端执行语义的工具才读取目标字段；全局软件操作工具不因一个
/// 同名的 `session_id` 字段被误判为远端执行。
pub fn target_argument_for_tool<'a>(tool_name: &str, parsed: &'a Value) -> Option<&'a str> {
    let field = match tool_name {
        "remote_exec" | "remote_fs" | "transfer_file" | "container_workspace" => "target",
        "host_monitor" | "port_forward_ops" | "sftp_transfer" => "session_id",
        _ => return None,
    };
    parsed[field]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::target_argument_for_tool;

    #[test]
    fn extracts_only_runtime_targets() {
        let args = json!({ "target": "prod-01", "session_id": "saved-01" });
        assert_eq!(
            target_argument_for_tool("remote_exec", &args),
            Some("prod-01")
        );
        assert_eq!(
            target_argument_for_tool("remote_fs", &args),
            Some("prod-01")
        );
        assert_eq!(
            target_argument_for_tool("host_monitor", &args),
            Some("saved-01")
        );
        assert_eq!(target_argument_for_tool("app_settings_ops", &args), None);
    }
}
