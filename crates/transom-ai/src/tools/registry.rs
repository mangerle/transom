//! 工具注册表与反向 MCP Schema 统一导出。

use std::collections::HashMap;
use std::sync::LazyLock;

use parking_lot::Mutex;

use super::{
    ToolRuntimeContext, app_tools, app_tools_dev, compare, delegate, extra, ops, plan, remote,
    skill_tool,
};
use crate::compaction::estimate_tokens;
use transom_ai_contracts::policy::ToolRisk;
use transom_ai_contracts::session::SessionScope;

/// 构造单个工具的 MCP JSON Schema 条目。
fn mcp_schema_entry<T: rig_agent::tool::Tool>(tool: &T) -> serde_json::Value {
    serde_json::json!({
        "name": T::NAME,
        "description": tool.description(),
        "inputSchema": tool.parameters(),
    })
}

/// 当前运行上下文对应的工具装配计划。
///
/// 注册与 Token 估算共用这份布尔计划，避免工具可用条件在两个入口漂移。
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ToolRegistrationPlan {
    knowledge_search: bool,
    subagent: bool,
    skill_loader: bool,
    app_bridge: bool,
    remote_exec: bool,
    remote_fs: bool,
    host_tools: bool,
    bulk_tools: bool,
}

impl ToolRegistrationPlan {
    fn from_context(ctx: &ToolRuntimeContext) -> Self {
        let app_bridge = ctx.caps.app_bridge.is_some();
        Self {
            knowledge_search: ctx.caps.knowledge_searcher.is_some(),
            subagent: ctx.caps.subagent_runner.is_some(),
            skill_loader: ctx.caps.skill_loader.is_some(),
            app_bridge,
            remote_exec: app_bridge || ctx.caps.remote_exec.is_some(),
            remote_fs: app_bridge || ctx.caps.remote_fs.is_some(),
            host_tools: matches!(ctx.scope, SessionScope::Host(_)) && ctx.locked_session.is_some(),
            bulk_tools: ctx.bulk_hosts.len() >= 2,
        }
    }
}

static TOOL_SCHEMA_TOKEN_CACHE: LazyLock<Mutex<HashMap<ToolRegistrationPlan, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

/// 按实际装配计划构造当前运行可用的工具 Schema。
fn context_tool_schemas(ctx: &ToolRuntimeContext) -> Vec<serde_json::Value> {
    let plan = ToolRegistrationPlan::from_context(ctx);
    let mut schemas = Vec::with_capacity(24);
    schemas.push(mcp_schema_entry(&extra::ReadToolOutputTool::new(
        ctx.clone(),
    )));
    schemas.push(mcp_schema_entry(&plan::UpdatePlanTool::new(ctx.clone())));
    schemas.push(mcp_schema_entry(&plan::RequestUserInputTool::new(
        ctx.clone(),
    )));
    if plan.knowledge_search {
        schemas.push(mcp_schema_entry(&plan::SearchKnowledgeTool::new(
            ctx.clone(),
        )));
    }
    if plan.subagent {
        schemas.push(mcp_schema_entry(&delegate::DelegateTaskTool::new(
            ctx.clone(),
        )));
    }
    if plan.skill_loader {
        schemas.push(mcp_schema_entry(&skill_tool::LoadSkillTool::new(
            ctx.clone(),
        )));
    }
    if plan.app_bridge {
        macro_rules! push_app_schemas {
            ($($tool_ty:path),+ $(,)?) => {
                $(schemas.push(mcp_schema_entry(&<$tool_ty>::new(ctx.clone())));)+
            };
        }
        for_each_app_bridge_tool!(push_app_schemas);
    }
    if plan.remote_exec {
        schemas.push(mcp_schema_entry(&remote::RemoteExecTool::new(ctx.clone())));
    }
    if plan.remote_fs {
        schemas.push(mcp_schema_entry(&ops::RemoteFsTool::new(ctx.clone())));
        schemas.push(mcp_schema_entry(&ops::TransferFileTool::new(ctx.clone())));
    }
    if plan.host_tools {
        schemas.push(mcp_schema_entry(&remote::GetRecentBlocksTool::new(
            ctx.clone(),
        )));
        schemas.push(mcp_schema_entry(&remote::ReadTerminalTool::new(
            ctx.clone(),
        )));
        schemas.push(mcp_schema_entry(&remote::ProposeHostMemoryTool::new(
            ctx.clone(),
        )));
        schemas.push(mcp_schema_entry(&ops::InspectHostRuntimeTool::new(
            ctx.clone(),
        )));
        schemas.push(mcp_schema_entry(&ops::ContainerOpsTool::new(ctx.clone())));
        schemas.push(mcp_schema_entry(&extra::PortForwardTool::new(ctx.clone())));
    }
    if plan.bulk_tools {
        schemas.push(mcp_schema_entry(&extra::RemoteExecBulkTool::new(
            ctx.clone(),
        )));
        schemas.push(mcp_schema_entry(&compare::CompareHostsTool::new(
            ctx.clone(),
        )));
    }
    schemas
}

/// 估算当前运行实际装配的工具 Schema Token 占用。
///
/// Schema 只由装配计划决定，因此在计划相同的情况下复用估算结果，避免每轮
/// 对话重复构造并序列化全部工具定义。
pub fn estimate_tool_schema_tokens(ctx: &ToolRuntimeContext) -> u32 {
    let plan = ToolRegistrationPlan::from_context(ctx);
    if let Some(tokens) = TOOL_SCHEMA_TOKEN_CACHE.lock().get(&plan).copied() {
        return tokens;
    }
    let schemas = context_tool_schemas(ctx);
    let tokens = serde_json::to_string(&schemas).map_or_else(
        |err| {
            log::warn!("序列化工具 Schema 以估算上下文占用失败: {err}");
            0
        },
        |serialized| estimate_tokens(&serialized),
    );
    TOOL_SCHEMA_TOKEN_CACHE.lock().insert(plan, tokens);
    tokens
}

/// 按当前作用域动态装配注册给 `rig::AgentBuilder` 的工具。
pub fn attach_tools(
    mut builder: rig_agent::AgentBuilder,
    ctx: &ToolRuntimeContext,
) -> rig_agent::AgentBuilder {
    let plan = ToolRegistrationPlan::from_context(ctx);
    builder = builder
        .tool(extra::ReadToolOutputTool::new(ctx.clone()))
        .tool(plan::UpdatePlanTool::new(ctx.clone()))
        .tool(plan::RequestUserInputTool::new(ctx.clone()));
    if plan.knowledge_search {
        builder = builder.tool(plan::SearchKnowledgeTool::new(ctx.clone()));
    }
    if plan.subagent {
        builder = builder.tool(delegate::DelegateTaskTool::new(ctx.clone()));
    }
    if plan.skill_loader {
        builder = builder.tool(skill_tool::LoadSkillTool::new(ctx.clone()));
    }
    if plan.app_bridge {
        macro_rules! attach_app_tools {
            ($($tool_ty:path),+ $(,)?) => {
                $(builder = builder.tool(<$tool_ty>::new(ctx.clone()));)+
            };
        }
        for_each_app_bridge_tool!(attach_app_tools);
    }
    if plan.remote_exec {
        builder = builder.tool(remote::RemoteExecTool::new(ctx.clone()));
    }
    if plan.remote_fs {
        builder = builder
            .tool(ops::RemoteFsTool::new(ctx.clone()))
            .tool(ops::TransferFileTool::new(ctx.clone()));
    }
    if plan.host_tools {
        builder = builder
            .tool(remote::GetRecentBlocksTool::new(ctx.clone()))
            .tool(remote::ReadTerminalTool::new(ctx.clone()))
            .tool(remote::ProposeHostMemoryTool::new(ctx.clone()))
            .tool(ops::InspectHostRuntimeTool::new(ctx.clone()))
            .tool(ops::ContainerOpsTool::new(ctx.clone()))
            .tool(extra::PortForwardTool::new(ctx.clone()));
    }
    if plan.bulk_tools {
        builder = builder
            .tool(extra::RemoteExecBulkTool::new(ctx.clone()))
            .tool(compare::CompareHostsTool::new(ctx.clone()));
    }
    builder
}

#[cfg(test)]
#[test]
fn every_registered_app_action_has_a_shared_risk() {
    use super::AppBridgeDomain;
    use super::app_bridge_types::validate_software_action;

    let empty_args = serde_json::json!({});
    macro_rules! assert_actions {
        ($($tool_ty:path),+ $(,)?) => {
            $(
                assert_eq!(
                    <$tool_ty>::NAME,
                    <$tool_ty as rig_agent::tool::Tool>::NAME
                );
                let domain = AppBridgeDomain::parse(<$tool_ty>::NAME)
                    .expect("软件工具必须有对应的 AppBridgeDomain");
                for action in <$tool_ty>::ACTIONS {
                    assert!(
                        app_tool_action_risk(<$tool_ty>::NAME, action).is_some(),
                        "工具 {} 的动作 {action} 缺少风险定义",
                        <$tool_ty>::NAME
                    );
                    assert!(
                        validate_software_action(domain, action, &empty_args).is_ok(),
                        "工具 {} 的动作 {action} 缺少强类型 Action 枚举定义",
                        <$tool_ty>::NAME
                    );
                }
            )+
        };
    }
    for_each_app_bridge_tool!(assert_actions);
}

#[cfg(test)]
#[test]
fn tool_schema_estimate_uses_the_actual_registration_plan() {
    let ctx = ToolRuntimeContext::dummy_for_schema();
    let schemas = context_tool_schemas(&ctx);
    assert_eq!(schemas.len(), 3, "全局最小上下文只装配三个基础工具");

    let first = estimate_tool_schema_tokens(&ctx);
    let second = estimate_tool_schema_tokens(&ctx);
    assert!(first > 0, "动态 Schema 估算不得为零");
    assert_eq!(first, second, "相同装配计划必须命中缓存");
}
