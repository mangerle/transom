//! 软件全局领域工具的动态风险评估与审批卡片摘要。

use serde_json::Value;

use super::{ApprovalSummary, ResolvedToolTarget, ToolRiskAssessment};
use crate::policy::ToolRisk;

/// 评估软件操作领域工具（10 大领域）的风险等级与确认摘要；非软件领域工具返回 `None`。
pub fn assess_app_tool(
    tool_name: &str,
    parsed: &Value,
    args_json: &str,
    target: &ResolvedToolTarget,
) -> Option<ToolRiskAssessment> {
    let action = parsed["action"].as_str().unwrap_or("");
    let risk = super::registry::app_tool_action_risk(tool_name, action)?;

    let title = format!("软件操作 {tool_name} · {action}");
    let detail = if tool_name == "app_settings_ops" && parsed.get("passphrase").is_some() {
        "包含口令参数（已脱敏）".to_owned()
    } else {
        args_json.to_owned()
    };
    let force_confirm = risk == ToolRisk::Destructive
        || (tool_name == "container_workspace"
            && matches!(action, "create_and_start_container" | "deploy_compose_yaml"));
    let host_label = target.label.clone();

    Some(ToolRiskAssessment {
        risk,
        force_confirm,
        summary: ApprovalSummary {
            tool_name: tool_name.to_owned(),
            host_label,
            protection: target.protection,
            risk,
            title,
            detail,
            diff_preview: None,
            requires_fresh_approval: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::assess_app_tool;
    use crate::policy::{HostProtection, ToolRisk};
    use crate::tools::ResolvedToolTarget;

    #[test]
    fn classifies_app_tools_risk_tiers() {
        let cases = [
            (
                "session_catalog",
                r#"{"action":"list"}"#,
                ToolRisk::SafeRead,
            ),
            ("session_catalog", r#"{"action":"create"}"#, ToolRisk::Write),
            (
                "session_catalog",
                r#"{"action":"delete"}"#,
                ToolRisk::Destructive,
            ),
            (
                "workspace_control",
                r#"{"action":"connect_session"}"#,
                ToolRisk::Write,
            ),
            (
                "workspace_control",
                r#"{"action":"send_terminal_input"}"#,
                ToolRisk::Destructive,
            ),
            (
                "host_monitor",
                r#"{"action":"list_processes"}"#,
                ToolRisk::SafeRead,
            ),
            (
                "host_monitor",
                r#"{"action":"kill_process"}"#,
                ToolRisk::Destructive,
            ),
            (
                "container_workspace",
                r#"{"action":"remove_container"}"#,
                ToolRisk::Destructive,
            ),
            (
                "git_workspace",
                r#"{"action":"status"}"#,
                ToolRisk::SafeRead,
            ),
            (
                "git_workspace",
                r#"{"action":"discard"}"#,
                ToolRisk::Destructive,
            ),
            (
                "ai_memory_skills",
                r#"{"action":"append_memory"}"#,
                ToolRisk::Write,
            ),
            (
                "container_workspace",
                r#"{"action":"create_and_start_container","image":"nginx:alpine","target":"prod-01"}"#,
                ToolRisk::Write,
            ),
            (
                "container_workspace",
                r#"{"action":"deploy_compose_yaml","yaml":"services:\n  web:\n    image: nginx"}"#,
                ToolRisk::Write,
            ),
        ];
        for (tool, args, expected) in cases {
            let parsed: serde_json::Value = serde_json::from_str(args).unwrap();
            let target = ResolvedToolTarget::new(None, "测试主机", HostProtection::Normal);
            let got = assess_app_tool(tool, &parsed, args, &target).expect("必须命中评估");
            assert_eq!(got.risk, expected, "tool={tool} args={args}");
        }
    }
}
