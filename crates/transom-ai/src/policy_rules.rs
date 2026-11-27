//! Shell 命令阻塞检测与静态风险分级规则表。

use crate::policy::ToolRisk;

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

    if lower.contains('>') || lower.contains("$(") || lower.contains('`') {
        return ToolRisk::Write;
    }

    if accesses_sensitive_credential_path(&lower) {
        return ToolRisk::SensitiveRead;
    }

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
