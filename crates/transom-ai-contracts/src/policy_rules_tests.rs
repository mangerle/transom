use super::{ShellCommandAssessment, assess_shell_command, detect_blocking_command};
use crate::policy::ToolRisk;

fn assert_safe(command: &str) {
    assert_eq!(
        assess_shell_command(command),
        ShellCommandAssessment {
            risk: ToolRisk::SafeRead,
            requires_confirmation: false,
        },
        "命令应被识别为安全只读: {command}"
    );
}

fn assert_uncertain(command: &str) {
    let assessment = assess_shell_command(command);
    assert_eq!(
        assessment.risk,
        ToolRisk::Write,
        "未知命令必须按写风险处理: {command}"
    );
    assert!(
        assessment.requires_confirmation,
        "未知命令必须强制确认: {command}"
    );
}

#[test]
fn safe_readonly_commands_are_strictly_allowlisted() {
    assert_safe("df -h && free -m");
    assert_safe("systemctl status nginx | head -n 20");
    assert_safe("journalctl -n 200 --no-pager");
    assert_safe("docker ps -a");
    assert_safe("ip addr show");
    assert_safe("date '+%F %T'");
}

#[test]
fn mutating_or_executable_commands_are_not_treated_as_safe_read() {
    assert_uncertain("curl -o /tmp/body https://example.com");
    assert_uncertain("awk 'BEGIN{system(\"touch /tmp/pwn\")}'");
    assert_uncertain("sort -o /tmp/sorted /var/log/syslog");
    assert_uncertain("env touch /tmp/pwn");
    assert_uncertain("git status");
    assert_uncertain("unknown-command --check");
}

#[test]
fn known_writes_remain_visible_but_do_not_require_extra_confirmation() {
    assert_eq!(
        assess_shell_command("systemctl restart nginx"),
        ShellCommandAssessment {
            risk: ToolRisk::Write,
            requires_confirmation: false,
        }
    );
    assert_eq!(
        assess_shell_command("rm -rf /var/tmp/x").risk,
        ToolRisk::Destructive
    );
}

#[test]
fn sensitive_paths_are_not_treated_as_plain_safe_reads() {
    assert_eq!(
        assess_shell_command("cat ~/.ssh/id_ed25519").risk,
        ToolRisk::SensitiveRead
    );
    assert_eq!(
        assess_shell_command("cat /etc/shadow").risk,
        ToolRisk::SensitiveRead
    );
}

#[test]
fn encoded_destructive_commands_are_not_downgraded() {
    assert_eq!(
        assess_shell_command("r''m -rf /var/tmp/x").risk,
        ToolRisk::Destructive
    );
    assert_eq!(
        assess_shell_command("rm -r /var/tmp/x").risk,
        ToolRisk::Destructive
    );
    assert_eq!(
        assess_shell_command("dd of=/tmp/x if=/dev/zero").risk,
        ToolRisk::Destructive
    );
}

#[test]
fn dangerous_subcommands_and_flags_are_not_auto_allowed() {
    assert_eq!(
        assess_shell_command("journalctl --vacuum-time=1d").risk,
        ToolRisk::Write
    );
    assert_eq!(
        assess_shell_command("date -s '2026-01-01'").risk,
        ToolRisk::Write
    );
    assert_eq!(
        assess_shell_command("ip link set eth0 down").risk,
        ToolRisk::Write
    );
    assert_eq!(
        assess_shell_command("ifconfig eth0 down").risk,
        ToolRisk::Write
    );
}

#[test]
fn blocking_guard_catches_streams_and_container_stats() {
    assert!(detect_blocking_command("tail -f /var/log/syslog").is_some());
    assert!(detect_blocking_command("docker compose logs -f api").is_some());
    assert!(detect_blocking_command("docker stats").is_some());
    assert!(detect_blocking_command("kubectl get pods --watch").is_some());
    assert!(detect_blocking_command("tail -n 200 /var/log/syslog").is_none());
    assert!(detect_blocking_command("echo start & tail -f /var/log/syslog").is_some());
    assert!(detect_blocking_command("docker stats --no-stream").is_none());
}

#[test]
fn background_commands_are_not_classified_as_safe_reads() {
    assert_eq!(
        assess_shell_command("ls & touch /tmp/x").risk,
        ToolRisk::Write
    );
    assert_eq!(
        assess_shell_command("echo ok & systemctl restart nginx").risk,
        ToolRisk::Write
    );
    assert_eq!(
        assess_shell_command("grep 'a & b' /tmp/log").risk,
        ToolRisk::SafeRead
    );
}

#[test]
fn input_redirection_is_never_auto_approved() {
    let assessment = assess_shell_command("grep root < /etc/passwd");
    assert_eq!(assessment.risk, ToolRisk::Write);
    assert!(assessment.requires_confirmation);
}
