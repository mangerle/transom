use super::{
    CommandPolicyContext, ExecutionMode, HostProtection, PolicyDecision, ToolRisk,
    classify_shell_command, detect_blocking_command, evaluate_bulk_policy, evaluate_command_policy,
    evaluate_policy,
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

    assert_eq!(
        evaluate_bulk_policy(
            &[HostProtection::Normal],
            ExecutionMode::AllowAll,
            ToolRisk::Destructive,
        ),
        (PolicyDecision::RequireApproval, true)
    );
    assert_eq!(
        evaluate_bulk_policy(
            &[HostProtection::Normal],
            ExecutionMode::Auto,
            ToolRisk::SensitiveRead,
        ),
        (PolicyDecision::RequireApproval, false)
    );
    assert_eq!(
        evaluate_bulk_policy(
            &[HostProtection::Normal],
            ExecutionMode::AllowAll,
            ToolRisk::SensitiveRead,
        ),
        (PolicyDecision::Allow, false)
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

#[test]
fn unknown_commands_require_confirmation_even_in_auto_mode() {
    use crate::approval::{ApprovalGrantStore, ApprovalScope};

    let grants = ApprovalGrantStore::new();
    let scope = ApprovalScope::new("chat-1", Some("host-1".to_owned()), HostProtection::Normal);
    let decision = evaluate_command_policy(&CommandPolicyContext {
        protection: HostProtection::Normal,
        mode: ExecutionMode::Auto,
        plan_mode: false,
        base_risk: ToolRisk::Write,
        force_confirm: false,
        command: Some("curl -o /tmp/body https://example.com"),
        exec_policy_rules: &[],
        approval_scope: &scope,
        approval_grants: &grants,
    });
    assert_eq!(decision, PolicyDecision::RequireApproval);
}
