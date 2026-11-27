//! 工具调用动态风险评估与审批摘要生成。

use super::assess_app::assess_app_tool;
use super::{ApprovalSummary, ToolRiskAssessment, ToolRuntimeContext};
use crate::policy::{HostProtection, ToolRisk, classify_shell_command};

/// 解析工具调用的目标显示标签（支持 `target = "local"` 或跨会话目标名）。
pub(crate) fn resolve_target_label(parsed: &serde_json::Value, ctx: &ToolRuntimeContext) -> String {
    match parsed["target"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(t) if t.eq_ignore_ascii_case("local") => "本机 (Local)".to_owned(),
        Some(t) => t.to_owned(),
        None => ctx.host_label(),
    }
}

/// 根据工具名与参数 JSON 字符串动态判定本次调用的风险级别与审批卡片摘要。
pub fn assess_tool_call(
    tool_name: &str,
    args_json: &str,
    ctx: &ToolRuntimeContext,
) -> ToolRiskAssessment {
    let parsed: serde_json::Value =
        serde_json::from_str(args_json).unwrap_or(serde_json::Value::Null);
    assess_tool_call_value(tool_name, &parsed, args_json, ctx)
}

/// 基于已解析的参数 `Value` 动态判定工具风险与审批摘要（避免调用方重复解析 JSON）。
pub fn assess_tool_call_value(
    tool_name: &str,
    parsed: &serde_json::Value,
    args_json: &str,
    ctx: &ToolRuntimeContext,
) -> ToolRiskAssessment {
    let host_label = resolve_target_label(parsed, ctx);
    let protection = ctx.protection();

    match tool_name {
        "remote_exec" => {
            let cmd = parsed["command"].as_str().unwrap_or(args_json);
            let risk = classify_shell_command(cmd);
            let is_local = parsed["target"]
                .as_str()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("local"));
            let title = if is_local {
                "执行本机 Shell 命令".to_owned()
            } else {
                "执行远端 Shell 命令".to_owned()
            };
            let detail = match parsed["cwd"].as_str().filter(|s| !s.trim().is_empty()) {
                Some(cwd) => format!("工作目录: {cwd}\n命令: {cmd}"),
                None => cmd.to_owned(),
            };
            ToolRiskAssessment {
                risk,
                force_confirm: false,
                summary: ApprovalSummary {
                    tool_name: tool_name.to_owned(),
                    host_label,
                    protection,
                    risk,
                    title,
                    detail,
                    diff_preview: None,
                    requires_fresh_approval: false,
                },
            }
        }
        "remote_exec_bulk" => assess_bulk_exec(parsed, args_json, ctx),
        "delegate_task" => {
            let mode = parsed["mode"].as_str().unwrap_or("readonly");
            let risk = if mode == "deploy" {
                ToolRisk::Write
            } else {
                ToolRisk::SafeRead
            };
            let title = parsed["title"].as_str().unwrap_or("子任务");
            ToolRiskAssessment {
                risk,
                force_confirm: false,
                summary: ApprovalSummary {
                    tool_name: tool_name.to_owned(),
                    host_label,
                    protection,
                    risk,
                    title: format!("派发子任务 Agent · {title}"),
                    detail: parsed["task"].as_str().unwrap_or(args_json).to_owned(),
                    diff_preview: None,
                    requires_fresh_approval: false,
                },
            }
        }
        "read_tool_output" | "update_plan" | "request_user_input" | "search_knowledge"
        | "load_skill" => ToolRiskAssessment {
            risk: ToolRisk::SafeRead,
            force_confirm: false,
            summary: ApprovalSummary {
                tool_name: tool_name.to_owned(),
                host_label,
                protection,
                risk: ToolRisk::SafeRead,
                title: format!("只读辅助工具: {tool_name}"),
                detail: args_json.to_owned(),
                diff_preview: None,
                requires_fresh_approval: false,
            },
        },
        "get_recent_blocks" => ToolRiskAssessment {
            risk: ToolRisk::SafeRead,
            force_confirm: false,
            summary: ApprovalSummary {
                tool_name: tool_name.to_owned(),
                host_label,
                protection,
                risk: ToolRisk::SafeRead,
                title: "读取最近命令块".to_owned(),
                detail: "读取当前会话最近的 OSC 133 命令执行记录".to_owned(),
                diff_preview: None,
                requires_fresh_approval: false,
            },
        },
        "read_terminal" => ToolRiskAssessment {
            risk: ToolRisk::SensitiveRead,
            force_confirm: true,
            summary: ApprovalSummary {
                tool_name: tool_name.to_owned(),
                host_label,
                protection,
                risk: ToolRisk::SensitiveRead,
                title: "读取终端屏幕输出".to_owned(),
                detail: format!(
                    "读取当前终端末尾 {} 行内容",
                    parsed["lines"].as_u64().unwrap_or(80)
                ),
                diff_preview: None,
                requires_fresh_approval: false,
            },
        },
        "propose_host_memory" => ToolRiskAssessment {
            risk: ToolRisk::Write,
            force_confirm: true,
            summary: ApprovalSummary {
                tool_name: tool_name.to_owned(),
                host_label,
                protection,
                risk: ToolRisk::Write,
                title: "记录主机长期记忆".to_owned(),
                detail: parsed["fact"].as_str().unwrap_or("").to_owned(),
                diff_preview: None,
                requires_fresh_approval: false,
            },
        },
        "remote_fs" => {
            let action = parsed["action"].as_str().unwrap_or("read");
            let path = parsed["path"].as_str().unwrap_or("");
            let risk = match action {
                "delete" => ToolRisk::Destructive,
                "edit" | "mkdir" | "rename" | "chmod" => ToolRisk::Write,
                _ => ToolRisk::SafeRead,
            };
            let is_write = risk != ToolRisk::SafeRead;
            ToolRiskAssessment {
                risk,
                force_confirm: is_write,
                summary: ApprovalSummary {
                    tool_name: tool_name.to_owned(),
                    host_label,
                    protection,
                    risk,
                    title: format!("远端文件操作: {action}"),
                    detail: format!("目标路径: {path}"),
                    diff_preview: parsed["content"].as_str().map(|c| {
                        format!("即将写入（共 {} 字节），执行前将自动备份原文件", c.len())
                    }),
                    requires_fresh_approval: false,
                },
            }
        }
        "transfer_file" => ToolRiskAssessment {
            risk: ToolRisk::Write,
            force_confirm: true,
            summary: ApprovalSummary {
                tool_name: tool_name.to_owned(),
                host_label,
                protection,
                risk: ToolRisk::Write,
                title: "SFTP 文件传输".to_owned(),
                detail: format!(
                    "{}: {} -> {}",
                    parsed["direction"].as_str().unwrap_or("transfer"),
                    parsed["local_path"].as_str().unwrap_or(""),
                    parsed["remote_path"].as_str().unwrap_or("")
                ),
                diff_preview: None,
                requires_fresh_approval: false,
            },
        },
        "inspect_host_runtime" => ToolRiskAssessment {
            risk: ToolRisk::SafeRead,
            force_confirm: false,
            summary: ApprovalSummary {
                tool_name: tool_name.to_owned(),
                host_label,
                protection,
                risk: ToolRisk::SafeRead,
                title: "采集主机运行状态快照".to_owned(),
                detail: "获取 CPU、内存、磁盘及容器概览".to_owned(),
                diff_preview: None,
                requires_fresh_approval: false,
            },
        },
        "container_ops" => {
            let action = parsed["action"].as_str().unwrap_or("list");
            let risk = if matches!(action, "list" | "inspect" | "logs" | "stats") {
                ToolRisk::SafeRead
            } else {
                ToolRisk::Write
            };
            ToolRiskAssessment {
                risk,
                force_confirm: risk == ToolRisk::Write,
                summary: ApprovalSummary {
                    tool_name: tool_name.to_owned(),
                    host_label,
                    protection,
                    risk,
                    title: format!("容器操作: {action}"),
                    detail: parsed["container_id"]
                        .as_str()
                        .unwrap_or("全部容器")
                        .to_owned(),
                    diff_preview: None,
                    requires_fresh_approval: false,
                },
            }
        }
        "port_forward" => {
            let action = parsed["action"].as_str().unwrap_or("list");
            let risk = if action == "list" {
                ToolRisk::SafeRead
            } else {
                ToolRisk::Write
            };
            ToolRiskAssessment {
                risk,
                force_confirm: risk == ToolRisk::Write,
                summary: ApprovalSummary {
                    tool_name: tool_name.to_owned(),
                    host_label,
                    protection,
                    risk,
                    title: format!("端口转发: {action}"),
                    detail: args_json.to_owned(),
                    diff_preview: None,
                    requires_fresh_approval: false,
                },
            }
        }
        other => {
            if let Some(assessed) = assess_app_tool(other, parsed, args_json, ctx) {
                return assessed;
            }
            ToolRiskAssessment {
                risk: ToolRisk::Write,
                force_confirm: true,
                summary: ApprovalSummary {
                    tool_name: tool_name.to_owned(),
                    host_label,
                    protection,
                    risk: ToolRisk::Write,
                    title: format!("外部/扩展工具: {tool_name}"),
                    detail: args_json.to_owned(),
                    diff_preview: None,
                    requires_fresh_approval: false,
                },
            }
        }
    }
}

fn assess_bulk_exec(
    parsed: &serde_json::Value,
    args_json: &str,
    ctx: &ToolRuntimeContext,
) -> ToolRiskAssessment {
    let cmd = parsed["command"].as_str().unwrap_or(args_json);
    let risk = classify_shell_command(cmd);
    let requested_names: Option<Vec<&str>> = parsed["host_names"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());
    let targets: Vec<_> = ctx
        .bulk_hosts
        .iter()
        .filter(|h| match &requested_names {
            Some(names) if !names.is_empty() => names.contains(&h.host_name.as_str()),
            _ => true,
        })
        .collect();

    let host_list = targets
        .iter()
        .map(|h| format!("{} ({}, {})", h.host_name, h.login, h.protection.label()))
        .collect::<Vec<_>>()
        .join("、");
    let worst_protection = if targets
        .iter()
        .any(|h| h.protection == HostProtection::ReadOnly)
    {
        HostProtection::ReadOnly
    } else if targets
        .iter()
        .any(|h| h.protection == HostProtection::Production)
    {
        HostProtection::Production
    } else {
        HostProtection::Normal
    };
    let is_write = matches!(risk, ToolRisk::Write | ToolRisk::Destructive);

    ToolRiskAssessment {
        risk,
        force_confirm: is_write,
        summary: ApprovalSummary {
            tool_name: "remote_exec_bulk".to_owned(),
            host_label: format!("批量 {} 台主机", targets.len()),
            protection: worst_protection,
            risk,
            title: "多主机批量执行 Shell 命令".to_owned(),
            detail: format!("目标主机: {host_list}\n命令: {cmd}"),
            diff_preview: None,
            requires_fresh_approval: is_write,
        },
    }
}
