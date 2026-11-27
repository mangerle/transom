//! 安全护栏、工具风险分级与审批策略引擎。
//!
//! 设计原理（Why）：
//! - 运维场景要求“只读巡检尽量免打扰、写操作有护栏、高危与生产环境强兜底”；
//! - 策略判定收敛为纯函数 [`evaluate_policy`]，可在脱离 UI 和网络的单测中穷举验证。

use serde::{Deserialize, Serialize};
pub use transom_core::HostProtection;

use crate::approval::{ApprovalGrantStore, ApprovalScope};

/// 运行中发送新消息的默认行为。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunningMessageBehavior {
    /// 排队等待：当前任务完成后按 FIFO 自动依次发送。
    #[default]
    Queue,
    /// 中途引导（Steer）：不停止当前命令，在下一个推理/工具节点前注入为补充指令。
    Steer,
    /// 立即打断并发送：取消当前运行并立即执行新消息。
    Interrupt,
}

impl RunningMessageBehavior {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queue => "排队等待",
            Self::Steer => "中途引导",
            Self::Interrupt => "打断并发送",
        }
    }
}

/// 全局/会话命令执行模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// 手动确认：除纯只读工具外均需人工确认（出厂默认）。
    #[default]
    Manual,
    /// 只读自动：纯只读探测命令（如 df/free/ps/systemctl status）自动放行，写操作仍审批。
    SafeReadAuto,
    /// 自动执行：常规写命令在非生产/非只读主机上自动放行，高危与传输仍强制确认。
    Auto,
    /// 全部允许：除只读保护主机外，自动放行所有读写、高危与外部 Agent 权限请求。
    AllowAll,
}

impl ExecutionMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Manual => "手动确认",
            Self::SafeReadAuto => "只读自动",
            Self::Auto => "自动执行",
            Self::AllowAll => "全部允许",
        }
    }
}

/// 工具调用的动态风险等级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    /// 纯只读且无隐私泄漏风险（如查询磁盘、负载、容器列表、OSC 133 块）。
    SafeRead,
    /// 只读但可能涉及隐私或敏感路径（如读取当前终端屏幕、读取密钥路径）。
    SensitiveRead,
    /// 会修改系统或配置状态（如写文件、重启服务、创建端口转发）。
    Write,
    /// 高危破坏性操作（如 rm -rf / mkfs / dd / shutdown / chmod -R）。
    Destructive,
}

impl ToolRisk {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SafeRead => "只读",
            Self::SensitiveRead => "敏感只读",
            Self::Write => "变更操作",
            Self::Destructive => "高危操作",
        }
    }
}

/// 策略引擎的最终裁决。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    /// 直接放行执行。
    Allow,
    /// 挂起等待用户通过审批卡片确认。
    RequireApproval,
    /// 硬拦截并向模型反馈原因。
    Deny { reason: String },
}

/// 根据主机保护级别、执行模式与工具风险给出策略裁决。
pub fn evaluate_policy(
    protection: HostProtection,
    mode: ExecutionMode,
    risk: ToolRisk,
    force_confirm: bool,
) -> PolicyDecision {
    if protection == HostProtection::ReadOnly
        && matches!(risk, ToolRisk::Write | ToolRisk::Destructive)
    {
        return PolicyDecision::Deny {
            reason: "当前主机处于只读保护模式，已拦截所有写操作与高危变更".to_owned(),
        };
    }

    if mode == ExecutionMode::AllowAll {
        return PolicyDecision::Allow;
    }

    if force_confirm || risk == ToolRisk::Destructive {
        return PolicyDecision::RequireApproval;
    }

    match (protection, mode, risk) {
        (_, _, ToolRisk::SafeRead) => match mode {
            ExecutionMode::Manual => PolicyDecision::RequireApproval,
            ExecutionMode::SafeReadAuto | ExecutionMode::Auto | ExecutionMode::AllowAll => {
                PolicyDecision::Allow
            }
        },
        (_, _, ToolRisk::SensitiveRead) => PolicyDecision::RequireApproval,
        (HostProtection::Production, _, ToolRisk::Write) => PolicyDecision::RequireApproval,
        (
            HostProtection::Normal,
            ExecutionMode::Auto | ExecutionMode::AllowAll,
            ToolRisk::Write,
        ) => PolicyDecision::Allow,
        (
            HostProtection::Normal,
            ExecutionMode::Manual | ExecutionMode::SafeReadAuto,
            ToolRisk::Write,
        ) => PolicyDecision::RequireApproval,
        (HostProtection::ReadOnly, _, _) | (_, _, ToolRisk::Destructive) => {
            PolicyDecision::RequireApproval
        }
    }
}

/// 多主机批量执行的最大允许主机数。
pub const MAX_BULK_TARGETS: usize = 20;

/// 针对多主机批量执行工具（`remote_exec_bulk`）的策略裁决。
///
/// 返回 `(裁决结果, 是否强制每次重新人工确认)`：
/// - 目标为空或超过 20 台直接拒绝；
/// - 写/高危操作若目标中包含任意只读保护主机，整批拒绝；
/// - 写/高危操作无论执行模式为何，均强制要求每次重新弹窗审批（`requires_fresh_approval = true`）。
pub fn evaluate_bulk_policy(
    targets: &[HostProtection],
    mode: ExecutionMode,
    risk: ToolRisk,
) -> (PolicyDecision, bool) {
    if targets.is_empty() || targets.len() > MAX_BULK_TARGETS {
        return (
            PolicyDecision::Deny {
                reason: format!(
                    "批量执行目标数量必须在 1..={MAX_BULK_TARGETS} 台之间（当前 {} 台）",
                    targets.len()
                ),
            },
            false,
        );
    }

    if matches!(risk, ToolRisk::Write | ToolRisk::Destructive) {
        if targets.contains(&HostProtection::ReadOnly) {
            return (
                PolicyDecision::Deny {
                    reason: "批量执行目标中包含只读保护主机，已整批拦截写操作与高危变更".to_owned(),
                },
                false,
            );
        }
        if mode == ExecutionMode::AllowAll {
            return (PolicyDecision::Allow, false);
        }
        return (PolicyDecision::RequireApproval, true);
    }

    let decision = match mode {
        ExecutionMode::Manual => PolicyDecision::RequireApproval,
        ExecutionMode::SafeReadAuto | ExecutionMode::Auto | ExecutionMode::AllowAll => {
            PolicyDecision::Allow
        }
    };
    (decision, false)
}

pub use crate::policy_rules::{
    BlockingCommandHit, classify_shell_command, detect_blocking_command,
    format_blocking_command_hint,
};

/// 统一策略裁决输入上下文（供内置 `Gate` 与外部 ACP 权限协商共用）。
pub struct CommandPolicyContext<'a> {
    pub protection: HostProtection,
    pub mode: ExecutionMode,
    pub plan_mode: bool,
    pub base_risk: ToolRisk,
    pub force_confirm: bool,
    pub command: Option<&'a str>,
    pub exec_policy_rules: &'a [crate::exec_policy::ExecPolicyRule],
    pub approval_scope: &'a ApprovalScope,
    pub approval_grants: &'a ApprovalGrantStore,
}

/// 统一计算单次工具/命令调用的策略裁决结果。
///
/// 依次执行：
/// 1. `plan_mode` 只读硬拦截；
/// 2. 命令链静态规则与会话前缀白名单评估（若提供 `command`）；
/// 3. 基础 [`evaluate_policy`] 裁决与非破坏性白名单放行。
pub fn evaluate_command_policy(ctx: &CommandPolicyContext<'_>) -> PolicyDecision {
    if ctx.plan_mode && matches!(ctx.base_risk, ToolRisk::Write | ToolRisk::Destructive) {
        return PolicyDecision::Deny {
            reason: "当前处于只读规划（Plan）模式，已拦截变更操作。请仅执行只读探测，并通过 `update_plan` 输出完整方案等待用户确认后再执行。".to_owned(),
        };
    }

    let mut effective_risk = ctx.base_risk;
    let mut chain_allowlisted = false;
    let mut force_ask = ctx.force_confirm;

    if let Some(cmd) = ctx.command {
        let chain = crate::exec_policy::evaluate_command_chain(
            cmd,
            ctx.exec_policy_rules,
            ctx.approval_grants,
            ctx.approval_scope,
        );
        if let Some(denied) = chain.denied_reason {
            return PolicyDecision::Deny { reason: denied };
        }
        if chain.worst_risk > effective_risk {
            effective_risk = chain.worst_risk;
        }
        chain_allowlisted = chain.all_allowlisted;
        force_ask = force_ask || chain.force_ask;
        if ctx.plan_mode && matches!(effective_risk, ToolRisk::Write | ToolRisk::Destructive) {
            return PolicyDecision::Deny {
                reason: "当前处于只读规划（Plan）模式，已拦截变更命令。".to_owned(),
            };
        }
    }

    if force_ask {
        return PolicyDecision::RequireApproval;
    }

    let base = evaluate_policy(ctx.protection, ctx.mode, effective_risk, false);
    if base == PolicyDecision::RequireApproval
        && chain_allowlisted
        && ctx.protection != HostProtection::ReadOnly
        && effective_risk != ToolRisk::Destructive
    {
        PolicyDecision::Allow
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandPolicyContext, ExecutionMode, HostProtection, PolicyDecision, ToolRisk,
        classify_shell_command, detect_blocking_command, evaluate_bulk_policy,
        evaluate_command_policy, evaluate_policy,
    };

    #[test]
    fn classify_shell_command_distinguishes_levels() {
        assert_eq!(
            classify_shell_command("df -h && free -m"),
            ToolRisk::SafeRead
        );
        assert_eq!(
            classify_shell_command("systemctl status nginx | head -n 20"),
            ToolRisk::SafeRead
        );
        assert_eq!(
            classify_shell_command("echo foo > /etc/test.conf"),
            ToolRisk::Write
        );
        assert_eq!(
            classify_shell_command("systemctl restart nginx"),
            ToolRisk::Write
        );
        assert_eq!(
            classify_shell_command("rm -rf /var/log/*"),
            ToolRisk::Destructive
        );
        assert_eq!(
            classify_shell_command("cat ~/.ssh/id_ed25519"),
            ToolRisk::SensitiveRead
        );
        assert_eq!(
            classify_shell_command("cat /etc/shadow"),
            ToolRisk::SensitiveRead
        );
    }

    #[test]
    fn policy_enforces_readonly_and_production_guards() {
        assert!(matches!(
            evaluate_policy(
                HostProtection::ReadOnly,
                ExecutionMode::Auto,
                ToolRisk::Write,
                false
            ),
            PolicyDecision::Deny { .. }
        ));
        assert_eq!(
            evaluate_policy(
                HostProtection::Production,
                ExecutionMode::Auto,
                ToolRisk::Write,
                false
            ),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            evaluate_policy(
                HostProtection::Normal,
                ExecutionMode::Auto,
                ToolRisk::Destructive,
                false
            ),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            evaluate_policy(
                HostProtection::Production,
                ExecutionMode::SafeReadAuto,
                ToolRisk::SafeRead,
                false
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn blocking_command_guard_catches_streams_and_allows_bounded() {
        assert!(detect_blocking_command("tail -f /var/log/syslog").is_some());
        assert!(detect_blocking_command("sudo journalctl -u nginx -f").is_some());
        assert!(detect_blocking_command("docker logs -f api").is_some());
        assert!(detect_blocking_command("top").is_some());
        assert!(detect_blocking_command("ping 1.1.1.1").is_some());
        assert!(detect_blocking_command("vim /etc/hosts").is_some());

        assert!(detect_blocking_command("tail -n 200 /var/log/syslog").is_none());
        assert!(detect_blocking_command("journalctl -n 200 --no-pager").is_none());
        assert!(detect_blocking_command("top -b -n 1 | head -n 30").is_none());
        assert!(detect_blocking_command("ping -c 4 1.1.1.1").is_none());
    }

    #[test]
    fn bulk_policy_denies_readonly_and_forces_fresh_approval_on_write() {
        let (dec, fresh) = evaluate_bulk_policy(
            &[HostProtection::Normal, HostProtection::ReadOnly],
            ExecutionMode::Auto,
            ToolRisk::Write,
        );
        assert!(matches!(dec, PolicyDecision::Deny { .. }));
        assert!(!fresh);

        let (dec, fresh) = evaluate_bulk_policy(
            &[HostProtection::Normal, HostProtection::Production],
            ExecutionMode::Auto,
            ToolRisk::Write,
        );
        assert_eq!(dec, PolicyDecision::RequireApproval);
        assert!(fresh);

        let (dec, fresh) = evaluate_bulk_policy(
            &[HostProtection::Normal, HostProtection::ReadOnly],
            ExecutionMode::SafeReadAuto,
            ToolRisk::SafeRead,
        );
        assert_eq!(dec, PolicyDecision::Allow);
        assert!(!fresh);

        assert_eq!(
            evaluate_policy(
                HostProtection::Production,
                ExecutionMode::AllowAll,
                ToolRisk::Destructive,
                true
            ),
            PolicyDecision::Allow
        );
        assert!(matches!(
            evaluate_policy(
                HostProtection::ReadOnly,
                ExecutionMode::AllowAll,
                ToolRisk::Write,
                false
            ),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn approval_grants_are_scoped_and_ask_rules_win() {
        use crate::approval::{ApprovalGrantStore, ApprovalScope};
        use crate::exec_policy::{ExecPolicyAction, ExecPolicyRule};

        let mut grants = ApprovalGrantStore::new();
        let normal_scope =
            ApprovalScope::new("chat-1", Some("host-1".to_owned()), HostProtection::Normal);
        assert!(grants.grant_prefix(&normal_scope, "systemctl restart"));

        let allowed = evaluate_command_policy(&CommandPolicyContext {
            protection: HostProtection::Normal,
            mode: ExecutionMode::Auto,
            plan_mode: false,
            base_risk: ToolRisk::Write,
            force_confirm: false,
            command: Some("systemctl restart nginx"),
            exec_policy_rules: &[],
            approval_scope: &normal_scope,
            approval_grants: &grants,
        });
        assert_eq!(allowed, PolicyDecision::Allow);

        let production_scope = ApprovalScope::new(
            "chat-1",
            Some("host-1".to_owned()),
            HostProtection::Production,
        );
        let cross_protection = evaluate_command_policy(&CommandPolicyContext {
            protection: HostProtection::Production,
            mode: ExecutionMode::Auto,
            plan_mode: false,
            base_risk: ToolRisk::Write,
            force_confirm: false,
            command: Some("systemctl restart nginx"),
            exec_policy_rules: &[],
            approval_scope: &production_scope,
            approval_grants: &grants,
        });
        assert_eq!(cross_protection, PolicyDecision::RequireApproval);

        let ask_rules = [ExecPolicyRule {
            id: "ask-restart".to_owned(),
            pattern: "systemctl restart".to_owned(),
            action: ExecPolicyAction::Ask,
            note: "生产服务重启需要人工确认".to_owned(),
            enabled: true,
        }];
        let ask_wins = evaluate_command_policy(&CommandPolicyContext {
            protection: HostProtection::Normal,
            mode: ExecutionMode::AllowAll,
            plan_mode: false,
            base_risk: ToolRisk::Write,
            force_confirm: false,
            command: Some("systemctl restart nginx"),
            exec_policy_rules: &ask_rules,
            approval_scope: &normal_scope,
            approval_grants: &grants,
        });
        assert_eq!(ask_wins, PolicyDecision::RequireApproval);
    }
}
