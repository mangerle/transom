//! 安全护栏、工具风险分级与审批策略引擎。
//!
//! 设计原理（Why）：
//! - 运维场景要求“只读巡检尽量免打扰、写操作有护栏、高危与生产环境强兜底”；
//! - 策略判定收敛为纯函数 [`evaluate_policy`]，可在脱离 UI 和网络的单测中穷举验证。

use serde::{Deserialize, Serialize};
pub use transom_core::HostProtection;

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

/// 命中的流式或交互式阻塞命令提示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockingCommandHit {
    pub command: &'static str,
    pub suggestion: &'static str,
}

/// 检测会持续阻塞或进入全屏交互界面的远端命令，避免无效等待整个执行超时。
pub fn detect_blocking_command(command: &str) -> Option<BlockingCommandHit> {
    let lower = command.trim().to_ascii_lowercase();
    for raw_seg in lower
        .split([';', '|'])
        .flat_map(|p| p.split("&&"))
        .flat_map(|p| p.split("||"))
    {
        let tokens: Vec<&str> = raw_seg.split_whitespace().collect();
        let mut idx = 0;
        if tokens.get(idx) == Some(&"sudo") {
            idx += 1;
        }
        let Some(&cmd) = tokens.get(idx) else {
            continue;
        };
        let rest = &tokens[idx + 1..];
        let has_flag = |f: &str| rest.contains(&f);
        let has_short_flag = |ch: char| {
            rest.iter()
                .any(|t| t.starts_with('-') && !t.starts_with("--") && t[1..].contains(ch))
        };

        match cmd {
            "tail" if has_short_flag('f') || has_short_flag('F') || has_flag("--follow") => {
                return Some(BlockingCommandHit {
                    command: "tail -f",
                    suggestion: "请改用非阻塞参数读取固定行数，例如 `tail -n 200 <文件>`",
                });
            }
            "journalctl" if has_short_flag('f') || has_flag("--follow") => {
                return Some(BlockingCommandHit {
                    command: "journalctl -f",
                    suggestion: "请改用 `journalctl -n 200 --no-pager`（或加 `--since \"10 minutes ago\"`）",
                });
            }
            "docker" | "podman" | "kubectl"
                if rest.first() == Some(&"logs")
                    && (has_short_flag('f') || has_flag("--follow")) =>
            {
                return Some(BlockingCommandHit {
                    command: "container logs -f",
                    suggestion: "请去掉 `-f` 并改用 `--tail 200`（或使用 `container_ops` 工具）",
                });
            }
            "top" if !has_short_flag('b') => {
                return Some(BlockingCommandHit {
                    command: "top",
                    suggestion: "交互式 `top` 会阻塞，请改用 `top -b -n 1 | head -n 30` 或 `inspect_host_runtime`",
                });
            }
            "htop" | "btop" | "atop" | "nmon" | "watch" => {
                return Some(BlockingCommandHit {
                    command: "fullscreen/watch",
                    suggestion: "全屏/轮询命令会一直阻塞，请改用单次快照命令（如 `ps aux`、`free -m`、`df -h`）",
                });
            }
            "vim" | "vi" | "nano" | "emacs" | "less" | "more" => {
                return Some(BlockingCommandHit {
                    command: "interactive_editor/pager",
                    suggestion: "交互式编辑器/分页器无法在无 TTY 通道使用，请改用 `remote_fs`（read/edit）或 `head`/`tail`/`sed -n`",
                });
            }
            "ping" if !has_short_flag('c') => {
                return Some(BlockingCommandHit {
                    command: "ping",
                    suggestion: "Linux 下未指定次数的 `ping` 会无限执行，请加上 `-c 4`（例如 `ping -c 4 <主机>`）",
                });
            }
            "tcpdump" if !has_short_flag('c') => {
                return Some(BlockingCommandHit {
                    command: "tcpdump",
                    suggestion: "未限制包数的 `tcpdump` 会无限阻塞，请加上 `-c 20`（或配合 `timeout 5s`）",
                });
            }
            _ => {}
        }
    }
    None
}

/// 生成返回给模型的阻塞命令拦截提示（不计为致命错误，引导其自动改写）。
pub fn format_blocking_command_hint(hit: &BlockingCommandHit) -> String {
    format!(
        "命令未发往远端：检测到阻塞/交互式命令（{}）。{}",
        hit.command, hit.suggestion
    )
}

/// 对将要执行的远端 Shell 命令进行静态风险分级。
///
/// 采用保守原则：仅当所有管道/分号片段均为明确的只读探测命令时才判为 `SafeRead`；
/// 出现重定向写入、反引号/命令替换或未知命令时至少按 `Write` 处理。
pub fn classify_shell_command(command: &str) -> ToolRisk {
    let normalized = command.trim();
    if normalized.is_empty() {
        return ToolRisk::Write;
    }

    let lower = normalized.to_ascii_lowercase();
    if is_destructive_pattern(&lower) {
        return ToolRisk::Destructive;
    }

    // 包含重定向写入、管道外危险构造或子命令替换时直接视作变更操作。
    if lower.contains('>') || lower.contains("$(") || lower.contains('`') {
        return ToolRisk::Write;
    }

    // 即使命令本身是只读的（如 cat / head），访问敏感凭据文件也升级为 SensitiveRead 强制审批。
    if accesses_sensitive_credential_path(&lower) {
        return ToolRisk::SensitiveRead;
    }

    // 拆解 `&&` / `;` / `|` 分段：全部片段均为只读白名单才判定为 SafeRead。
    let segments: Vec<&str> = lower
        .split([';', '|'])
        .flat_map(|part| part.split("&&"))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();

    if !segments.is_empty() && segments.iter().all(|seg| is_readonly_segment(seg)) {
        return ToolRisk::SafeRead;
    }

    ToolRisk::Write
}

fn accesses_sensitive_credential_path(lower: &str) -> bool {
    const CREDENTIAL_MARKERS: [&str; 6] = [
        "/etc/shadow",
        ".ssh/id_",
        ".aws/credentials",
        ".kube/config",
        "/.env",
        " .env",
    ];
    CREDENTIAL_MARKERS.iter().any(|m| lower.contains(m))
}

fn is_destructive_pattern(lower: &str) -> bool {
    const DESTRUCTIVE_MARKERS: [&str; 12] = [
        "rm -rf",
        "rm -fr",
        "mkfs",
        "dd if=",
        "shutdown",
        "reboot",
        "poweroff",
        "init 0",
        "init 6",
        "chmod -r 777",
        "iptables -f",
        ":(){:|:&};:",
    ];
    DESTRUCTIVE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn is_readonly_segment(seg: &str) -> bool {
    let mut parts = seg.split_whitespace();
    let Some(cmd) = parts.next() else {
        return false;
    };
    let sub = parts.next().unwrap_or("");

    match cmd {
        "ls" | "ll" | "pwd" | "whoami" | "id" | "uname" | "uptime" | "date" | "df" | "free"
        | "ps" | "ss" | "netstat" | "lsof" | "cat" | "head" | "tail" | "less" | "wc" | "stat"
        | "du" | "which" | "whereis" | "env" | "printenv" | "grep" | "egrep" | "rg" | "awk"
        | "sort" | "uniq" | "journalctl" | "dmesg" | "ip" | "ifconfig" | "ping" | "dig"
        | "nslookup" | "curl" => {
            // `curl` 只允许不带 `-X POST/PUT/DELETE` 或 `-d` 的简单探测
            if cmd == "curl" {
                return !seg.contains(" -x ")
                    && !seg.contains("--request")
                    && !seg.contains(" -d ")
                    && !seg.contains("--data");
            }
            true
        }
        "systemctl" => matches!(sub, "status" | "is-active" | "is-enabled" | "list-units"),
        "docker" | "podman" => {
            matches!(sub, "ps" | "logs" | "inspect" | "stats" | "images" | "info")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionMode, HostProtection, PolicyDecision, ToolRisk, classify_shell_command,
        detect_blocking_command, evaluate_bulk_policy, evaluate_policy,
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
}
