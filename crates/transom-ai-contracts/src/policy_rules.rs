//! Shell 命令阻塞检测与失败关闭式风险分级。
//!
//! 设计原理（Why）：
//! - 自动执行只允许“能够静态证明为只读”的命令；无法识别的命令必须进入人工确认，
//!   不能因为风险枚举落为 `Write` 就在非生产主机自动执行；
//! - 风险分级同时返回风险等级与“是否需要强制确认”，把未知性和破坏性分开表达；
//! - 子命令与危险参数使用严格白名单校验，避免 `curl -o`、`journalctl --vacuum-*`
//!   或 `awk 'BEGIN{system(...)}'` 被误判为只读。

use crate::exec_policy::split_shell_segments;
use crate::policy::ToolRisk;

/// 命中的流式或交互式阻塞命令提示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockingCommandHit {
    pub command: &'static str,
    pub suggestion: &'static str,
}

/// 单条 Shell 命令的风险评估结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellCommandAssessment {
    /// 静态风险等级。
    pub risk: ToolRisk,
    /// 即使执行模式允许常规写操作，也必须要求人工确认。
    pub requires_confirmation: bool,
}

impl ShellCommandAssessment {
    fn safe_read() -> Self {
        Self {
            risk: ToolRisk::SafeRead,
            requires_confirmation: false,
        }
    }

    fn known_write() -> Self {
        Self {
            risk: ToolRisk::Write,
            requires_confirmation: false,
        }
    }

    fn uncertain() -> Self {
        Self {
            risk: ToolRisk::Write,
            requires_confirmation: true,
        }
    }
}

/// 检测会持续阻塞或进入全屏交互界面的远端命令，避免无效等待整个执行超时。
pub fn detect_blocking_command(command: &str) -> Option<BlockingCommandHit> {
    let lower = command.trim().to_ascii_lowercase();
    for raw_seg in split_shell_segments(&lower) {
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
            "docker" | "podman"
                if rest.first() == Some(&"logs")
                    && (has_short_flag('f') || has_flag("--follow")) =>
            {
                return Some(BlockingCommandHit {
                    command: "container logs -f",
                    suggestion: "请去掉 `-f` 并改用 `--tail 200`（或使用 `container_ops` 工具）",
                });
            }
            "docker" | "podman"
                if rest.first() == Some(&"compose")
                    && rest.contains(&"logs")
                    && (has_short_flag('f') || has_flag("--follow")) =>
            {
                return Some(BlockingCommandHit {
                    command: "docker compose logs -f",
                    suggestion: "请去掉 `-f` 并改用 `--tail 200`",
                });
            }
            "docker" | "podman" if rest.first() == Some(&"stats") && !has_flag("--no-stream") => {
                return Some(BlockingCommandHit {
                    command: "container stats",
                    suggestion: "请增加 `--no-stream`，或使用 `container_ops` 的单次统计",
                });
            }
            "kubectl"
                if (rest.contains(&"logs") && (has_short_flag('f') || has_flag("--follow")))
                    || (rest.contains(&"get") && has_flag("--watch")) =>
            {
                return Some(BlockingCommandHit {
                    command: "kubectl follow/watch",
                    suggestion: "请改用单次查询，必要时增加 `--tail=200`",
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

/// 对将要执行的 Shell 命令执行失败关闭式静态风险评估。
///
/// 只有明确位于只读白名单且参数安全的命令才会返回 `SafeRead`。未知命令、命令替换、
/// 输出重定向或无法解析的词法结构都会返回 `Write + requires_confirmation`。
pub fn assess_shell_command(command: &str) -> ShellCommandAssessment {
    let normalized = command.trim();
    if normalized.is_empty() {
        return ShellCommandAssessment::uncertain();
    }

    let lower = normalized.to_ascii_lowercase();
    if is_destructive_pattern(&lower) {
        return ShellCommandAssessment {
            risk: ToolRisk::Destructive,
            requires_confirmation: false,
        };
    }
    if lower.contains("$(") || lower.contains('`') || lower.contains("<(") || lower.contains(">(") {
        return ShellCommandAssessment::uncertain();
    }
    if accesses_sensitive_credential_path(&lower) {
        return ShellCommandAssessment {
            risk: ToolRisk::SensitiveRead,
            requires_confirmation: false,
        };
    }

    let segments = split_shell_segments(normalized);
    if segments.is_empty() {
        return ShellCommandAssessment::uncertain();
    }

    let mut worst = ToolRisk::SafeRead;
    let mut requires_confirmation = false;
    for segment in &segments {
        let assessment = assess_segment(segment);
        if assessment.risk > worst {
            worst = assessment.risk;
        }
        requires_confirmation |= assessment.requires_confirmation;
    }

    ShellCommandAssessment {
        risk: worst,
        requires_confirmation,
    }
}

/// 对将执行的 Shell 命令进行静态风险分级，保留只返回风险等级的兼容入口。
pub fn classify_shell_command(command: &str) -> ToolRisk {
    assess_shell_command(command).risk
}

fn assess_segment(segment: &str) -> ShellCommandAssessment {
    if segment.contains('>') || segment.contains('<') {
        return ShellCommandAssessment::uncertain();
    }
    let Some(tokens) = parse_shell_words(segment) else {
        return ShellCommandAssessment::uncertain();
    };
    if tokens.iter().any(|token| is_shell_redirection(token)) {
        return ShellCommandAssessment::uncertain();
    }

    let mut idx = 0usize;
    while tokens.get(idx).is_some_and(|token| token == "sudo") {
        idx += 1;
    }
    while tokens
        .get(idx)
        .is_some_and(|token| token.contains('=') && !token.starts_with('-'))
    {
        idx += 1;
    }
    let Some(command) = tokens.get(idx) else {
        return ShellCommandAssessment::uncertain();
    };
    let args = &tokens[idx + 1..];
    if is_destructive_tokens(command, args) {
        return ShellCommandAssessment {
            risk: ToolRisk::Destructive,
            requires_confirmation: false,
        };
    }

    match command.as_str() {
        "ls" | "pwd" | "whoami" | "id" | "uname" | "uptime" | "df" | "free" | "ps" | "ss"
        | "netstat" | "lsof" | "cat" | "head" | "tail" | "wc" | "stat" | "du" | "which"
        | "whereis" | "printenv" | "grep" | "egrep" | "rg" | "ping" | "dig" | "nslookup"
        | "host" => ShellCommandAssessment::safe_read(),
        "date" if !has_date_write_arg(args) => ShellCommandAssessment::safe_read(),
        "date" => ShellCommandAssessment::known_write(),
        "dmesg" if !has_any_arg(args, &["-c", "-C", "--clear", "--read-clear"]) => {
            ShellCommandAssessment::safe_read()
        }
        "dmesg" => ShellCommandAssessment::known_write(),
        "journalctl" if !has_journal_write_flag(args) => ShellCommandAssessment::safe_read(),
        "journalctl" => ShellCommandAssessment::known_write(),
        "systemctl" => assess_systemctl(args),
        "docker" | "podman" => assess_container_cli(args),
        "ip" => assess_ip(args),
        "ifconfig" => assess_ifconfig(args),
        "rm" | "mv" | "cp" | "touch" | "mkdir" | "rmdir" | "chmod" | "chown" | "chgrp" | "ln"
        | "truncate" | "tee" | "install" | "apt" | "apt-get" | "yum" | "dnf" | "pacman"
        | "zypper" | "apk" | "cargo" | "npm" | "pnpm" | "yarn" | "pip" | "pip3" | "rsync"
        | "tar" | "unzip" | "zip" => ShellCommandAssessment::known_write(),
        _ => ShellCommandAssessment::uncertain(),
    }
}

fn assess_systemctl(args: &[String]) -> ShellCommandAssessment {
    let Some(subcommand) = args.iter().find(|arg| !arg.starts_with('-')) else {
        return ShellCommandAssessment::uncertain();
    };
    match subcommand.as_str() {
        "status" | "is-active" | "is-enabled" | "is-failed" | "list-units" | "list-unit-files"
        | "show" | "cat" => ShellCommandAssessment::safe_read(),
        "start" | "stop" | "restart" | "reload" | "enable" | "disable" | "mask" | "unmask"
        | "daemon-reload" => ShellCommandAssessment::known_write(),
        _ => ShellCommandAssessment::uncertain(),
    }
}

fn assess_container_cli(args: &[String]) -> ShellCommandAssessment {
    let Some(subcommand) = args.iter().find(|arg| !arg.starts_with('-')) else {
        return ShellCommandAssessment::uncertain();
    };
    match subcommand.as_str() {
        "ps" | "logs" | "inspect" | "stats" | "images" | "info" | "version" => {
            ShellCommandAssessment::safe_read()
        }
        "run" | "create" | "start" | "stop" | "restart" | "rm" | "rmi" | "pull" | "push"
        | "exec" | "compose" | "build" | "tag" | "login" | "logout" => {
            ShellCommandAssessment::known_write()
        }
        _ => ShellCommandAssessment::uncertain(),
    }
}

fn assess_ip(args: &[String]) -> ShellCommandAssessment {
    const WRITE_VERBS: [&str; 10] = [
        "add", "append", "change", "delete", "del", "flush", "replace", "set", "update", "exec",
    ];
    if has_any_arg(args, &WRITE_VERBS) {
        return ShellCommandAssessment::known_write();
    }
    const READ_VERBS: [&str; 14] = [
        "addr", "address", "a", "link", "l", "route", "r", "neigh", "neighbor", "n", "rule",
        "tunnel", "maddr", "help",
    ];
    if args.is_empty() || has_any_arg(args, &READ_VERBS) {
        ShellCommandAssessment::safe_read()
    } else {
        ShellCommandAssessment::uncertain()
    }
}

fn has_journal_write_flag(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--vacuum-size"
                | "--vacuum-time"
                | "--vacuum-files"
                | "--rotate"
                | "--flush"
                | "--sync"
                | "--relinquish-var"
                | "--setup-keys"
                | "--update-catalog"
        ) || arg.starts_with("--vacuum-size=")
            || arg.starts_with("--vacuum-time=")
            || arg.starts_with("--vacuum-files=")
    })
}

fn has_date_write_arg(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "-s" || arg == "--set" || arg.starts_with("--set=") || arg.starts_with("-s")
    })
}

fn assess_ifconfig(args: &[String]) -> ShellCommandAssessment {
    const WRITE_ARGS: [&str; 13] = [
        "up",
        "down",
        "add",
        "del",
        "delete",
        "change",
        "arp",
        "mtu",
        "netmask",
        "address",
        "broadcast",
        "pointopoint",
        "txqueuelen",
    ];
    let positional = args.iter().filter(|arg| !arg.starts_with('-')).count();
    if has_any_arg(args, &WRITE_ARGS) || positional > 1 {
        ShellCommandAssessment::known_write()
    } else {
        ShellCommandAssessment::safe_read()
    }
}

fn has_any_arg(args: &[String], candidates: &[&str]) -> bool {
    args.iter().any(|arg| candidates.contains(&arg.as_str()))
}

fn is_destructive_tokens(command: &str, args: &[String]) -> bool {
    match command {
        "rm" => args.iter().any(|arg| {
            arg == "--recursive"
                || (arg.starts_with('-')
                    && !arg.starts_with("--")
                    && (arg.contains('r') || arg.contains('R')))
        }),
        "chmod" => {
            args.iter().any(|arg| arg == "-R" || arg == "--recursive")
                && args.iter().any(|arg| arg == "777")
        }
        "dd" => args.iter().any(|arg| arg.starts_with("of=")),
        "mkfs" | "shutdown" | "reboot" | "poweroff" => true,
        _ => false,
    }
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
    CREDENTIAL_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn is_shell_redirection(token: &str) -> bool {
    matches!(
        token,
        ">" | ">>" | ">|" | "&>" | ">>&" | "&>>" | "<" | "<<" | "<<<" | "<>" | ">|<"
    )
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

fn parse_shell_words(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut has_token = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            has_token = true;
            escaped = false;
            continue;
        }
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    current.push(ch);
                }
                has_token = true;
            }
            Some('"') => match ch {
                '"' => quote = None,
                '\\' => escaped = true,
                _ => current.push(ch),
            },
            _ => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    has_token = true;
                }
                '\\' => {
                    escaped = true;
                    has_token = true;
                }
                ch if ch.is_whitespace() => {
                    if has_token {
                        words.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                }
                _ => {
                    current.push(ch);
                    has_token = true;
                }
            },
        }
    }

    if escaped || quote.is_some() {
        return None;
    }
    if has_token {
        words.push(current);
    }
    Some(words)
}

#[cfg(test)]
#[path = "policy_rules_tests.rs"]
mod tests;
