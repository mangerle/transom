//! 工具注册表与反向 MCP Schema 统一导出。

use super::{
    SessionScope, ToolRuntimeContext, app_tools, app_tools_dev, compare, delegate, extra, ops,
    plan, remote, skill_tool,
};
use crate::policy::ToolRisk;

/// 构造单个工具的 MCP JSON Schema 条目。
fn mcp_schema_entry<T: rig_agent::tool::Tool>(tool: &T) -> serde_json::Value {
    serde_json::json!({
        "name": T::NAME,
        "description": tool.description(),
        "inputSchema": tool.parameters(),
    })
}

/// 软件领域全局工具统一清单（`attach_tools` 与 `export_mcp_tool_schemas` 共用，杜绝双份维护漂移）。
macro_rules! for_each_app_bridge_tool {
    ($cb:ident) => {
        $cb!(
            app_tools::SessionCatalogTool,
            app_tools::WorkspaceControlTool,
            app_tools::SftpTransferTool,
            app_tools::HostMonitorTool,
            app_tools::PortForwardOpsTool,
            app_tools_dev::ContainerWorkspaceTool,
            app_tools_dev::GitWorkspaceTool,
            app_tools_dev::ApiClientTool,
            app_tools_dev::AppSettingsOpsTool,
            app_tools_dev::AiMemorySkillsTool,
        )
    };
}

/// 按工具名与动作读取软件工具的声明式风险，避免审批评估另维护一套 match。
pub(crate) fn app_tool_action_risk(tool_name: &str, action: &str) -> Option<ToolRisk> {
    macro_rules! lookup_risk {
        ($($tool_ty:path),+ $(,)?) => {
            $(
                if tool_name == <$tool_ty>::NAME {
                    return Some(<$tool_ty>::risk_for(action));
                }
            )+
        };
    }
    for_each_app_bridge_tool!(lookup_risk);
    None
}

/// 从真实 `Tool` 实现中统一导出反向 MCP Server 暴露的工具 Schema 清单，杜绝双份维护漂移。
pub(crate) fn export_mcp_tool_schemas() -> Vec<serde_json::Value> {
    let ctx = ToolRuntimeContext::dummy_for_schema();
    let mut schemas = vec![
        mcp_schema_entry(&remote::RemoteExecTool::new(ctx.clone())),
        mcp_schema_entry(&extra::RemoteExecBulkTool::new(ctx.clone())),
        mcp_schema_entry(&ops::RemoteFsTool::new(ctx.clone())),
        mcp_schema_entry(&ops::TransferFileTool::new(ctx.clone())),
        mcp_schema_entry(&ops::InspectHostRuntimeTool::new(ctx.clone())),
        mcp_schema_entry(&ops::ContainerOpsTool::new(ctx.clone())),
        mcp_schema_entry(&remote::GetRecentBlocksTool::new(ctx.clone())),
        mcp_schema_entry(&remote::ReadTerminalTool::new(ctx.clone())),
        mcp_schema_entry(&extra::ReadToolOutputTool::new(ctx.clone())),
    ];
    macro_rules! push_app_schemas {
        ($($tool_ty:path),+ $(,)?) => {
            $(schemas.push(mcp_schema_entry(&<$tool_ty>::new(ctx.clone())));)+
        };
    }
    for_each_app_bridge_tool!(push_app_schemas);
    schemas
}

/// 按当前作用域动态装配注册给 `rig::AgentBuilder` 的工具。
pub fn attach_tools(
    mut builder: rig_agent::AgentBuilder,
    ctx: &ToolRuntimeContext,
) -> rig_agent::AgentBuilder {
    builder = builder
        .tool(extra::ReadToolOutputTool::new(ctx.clone()))
        .tool(plan::UpdatePlanTool::new(ctx.clone()))
        .tool(plan::RequestUserInputTool::new(ctx.clone()));
    if ctx.caps.knowledge_searcher.is_some() {
        builder = builder.tool(plan::SearchKnowledgeTool::new(ctx.clone()));
    }
    if ctx.caps.subagent_runner.is_some() {
        builder = builder.tool(delegate::DelegateTaskTool::new(ctx.clone()));
    }
    if ctx.caps.skill_loader.is_some() {
        builder = builder.tool(skill_tool::LoadSkillTool::new(ctx.clone()));
    }
    if ctx.caps.app_bridge.is_some() {
        macro_rules! attach_app_tools {
            ($($tool_ty:path),+ $(,)?) => {
                $(builder = builder.tool(<$tool_ty>::new(ctx.clone()));)+
            };
        }
        for_each_app_bridge_tool!(attach_app_tools);
    }
    if ctx.caps.app_bridge.is_some() || ctx.caps.remote_exec.is_some() {
        builder = builder.tool(remote::RemoteExecTool::new(ctx.clone()));
    }
    if ctx.caps.app_bridge.is_some() || ctx.caps.remote_fs.is_some() {
        builder = builder
            .tool(ops::RemoteFsTool::new(ctx.clone()))
            .tool(ops::TransferFileTool::new(ctx.clone()));
    }
    if matches!(ctx.scope, SessionScope::Host(_)) && ctx.locked_session.is_some() {
        builder = builder
            .tool(remote::GetRecentBlocksTool::new(ctx.clone()))
            .tool(remote::ReadTerminalTool::new(ctx.clone()))
            .tool(remote::ProposeHostMemoryTool::new(ctx.clone()))
            .tool(ops::InspectHostRuntimeTool::new(ctx.clone()))
            .tool(ops::ContainerOpsTool::new(ctx.clone()))
            .tool(extra::PortForwardTool::new(ctx.clone()));
    }
    if ctx.bulk_hosts.len() >= 2 {
        builder = builder
            .tool(extra::RemoteExecBulkTool::new(ctx.clone()))
            .tool(compare::CompareHostsTool::new(ctx.clone()));
    }
    builder
}

#[cfg(test)]
#[test]
fn every_registered_app_action_has_a_shared_risk() {
    macro_rules! assert_actions {
        ($($tool_ty:path),+ $(,)?) => {
            $(
                assert_eq!(
                    <$tool_ty>::NAME,
                    <$tool_ty as rig_agent::tool::Tool>::NAME
                );
                for action in <$tool_ty>::ACTIONS {
                    assert!(
                        app_tool_action_risk(<$tool_ty>::NAME, action).is_some(),
                        "工具 {} 的动作 {action} 缺少风险定义",
                        <$tool_ty>::NAME
                    );
                }
            )+
        };
    }
    for_each_app_bridge_tool!(assert_actions);
}
