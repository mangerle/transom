//! 进程列表、监听端口、失败服务与 OOM 事件解析。

use transom_core::{FailedServiceInfo, OomAlert, PortListeningInfo, ProcessInfo};

/// 解析 `ps -eo pid,ppid,user,%cpu,%mem,rss,comm,args` 输出。
pub fn parse_processes(content: &str) -> Vec<ProcessInfo> {
    let mut processes = Vec::with_capacity(50);
    let has_args_header = content.lines().next().is_some_and(|l| l.contains("ARGS"));

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 7 {
            continue;
        }
        let Ok(pid) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(ppid) = parts[1].parse::<u32>() else {
            continue;
        };
        let Ok(cpu_percent) = parts[3].parse::<f32>() else {
            continue;
        };
        let Ok(mem_percent) = parts[4].parse::<f32>() else {
            continue;
        };
        let Ok(rss_kb) = parts[5].parse::<u64>() else {
            continue;
        };

        let (cmdline, args) = if has_args_header {
            let comm = parts[6].to_string();
            let args = if parts.len() > 7 {
                Some(parts[7..].join(" "))
            } else {
                None
            };
            (comm, args)
        } else {
            let full = parts[6..].join(" ");
            (full, None)
        };

        processes.push(ProcessInfo {
            pid,
            ppid,
            user: parts[2].to_string(),
            cpu_percent,
            mem_percent,
            rss_bytes: rss_kb.saturating_mul(1024),
            cmdline,
            args,
        });
    }
    processes
}

/// 统计 `ps -eo pid=` 输出的进程行数。
///
/// 折叠态只需要总数，用仅含 pid 的窄列输出把每轮传输量从全量明细的数十 KB
/// 压到数 KB，代价是计数与明细采样发生在不同时刻，两者可能相差一两个进程。
pub fn parse_process_count(content: &str) -> u32 {
    let lines = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    u32::try_from(lines).unwrap_or(u32::MAX)
}

/// 从 ss -tulpn 或 netstat -tulpn 解析监听端口
pub fn parse_ports(content: &str) -> Vec<PortListeningInfo> {
    let text = if let Some((_, rest)) = content.split_once("__PORTS__") {
        let end = rest.find("__").unwrap_or(rest.len());
        &rest[..end]
    } else if content.contains("__UPTIME__")
        || content.contains("__DF__")
        || content.contains("__NET__")
    {
        return Vec::new();
    } else {
        content
    };

    let mut ports = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Netid")
            || trimmed.starts_with("Active")
            || trimmed.starts_with("Proto")
        {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 4 {
            let protocol = parts[0].to_lowercase();
            if !protocol.starts_with("tcp") && !protocol.starts_with("udp") {
                continue;
            }
            let local_addr_str = parts
                .iter()
                .find(|p| p.contains(':') && !p.contains("users:"));
            if let Some(addr_str) = local_addr_str
                && let Some((addr, port_str)) = addr_str.rsplit_once(':')
                && let Ok(port) = port_str.parse::<u16>()
            {
                let mut pid = None;
                let mut process_name = None;
                if let Some(users_idx) = trimmed.find("users:((") {
                    let rest = &trimmed[users_idx + 8..];
                    let rest_trimmed = rest.trim_start_matches(['(', '"']);
                    if let Some(name_end) = rest_trimmed.find('"') {
                        process_name = Some(rest_trimmed[..name_end].to_string());
                    }
                    if let Some(pid_idx) = rest.find("pid=") {
                        let pid_str = &rest[pid_idx + 4..];
                        let pid_num: String =
                            pid_str.chars().take_while(|c| c.is_ascii_digit()).collect();
                        pid = pid_num.parse::<u32>().ok();
                    }
                } else if let Some(last) = parts.last()
                    && let Some((pid_s, name)) = last.split_once('/')
                {
                    pid = pid_s.parse::<u32>().ok();
                    process_name = Some(name.to_string());
                }
                if !ports
                    .iter()
                    .any(|p: &PortListeningInfo| p.protocol == protocol && p.port == port)
                {
                    ports.push(PortListeningInfo {
                        protocol,
                        bind_addr: addr.to_string(),
                        port,
                        pid,
                        process_name,
                    });
                }
            }
        }
    }
    ports.sort_by_key(|p| p.port);
    ports
}

/// 解析异常崩溃的 Systemd 服务
pub fn parse_failed_services(content: &str) -> Vec<FailedServiceInfo> {
    let text = if let Some((_, rest)) = content.split_once("__SERVICES__") {
        let end = rest.find("__").unwrap_or(rest.len());
        &rest[..end]
    } else if content.contains("__UPTIME__")
        || content.contains("__DF__")
        || content.contains("__NET__")
    {
        return Vec::new();
    } else {
        content
    };

    let mut services = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("0 loaded units listed")
            || trimmed.starts_with("UNIT")
        {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let start_idx = if parts.first() == Some(&"●") { 1 } else { 0 };
        if parts.len() >= start_idx + 4 {
            let unit = parts[start_idx].to_string();
            // Systemd 服务 unit 必然包含 '.' 且通常以 .service 结尾，防止其它非服务杂项行被误识别
            if !unit.contains('.') {
                continue;
            }
            let load = parts[start_idx + 1].to_string();
            let active = parts[start_idx + 2].to_string();
            let sub = parts[start_idx + 3].to_string();
            let description = if parts.len() > start_idx + 4 {
                parts[start_idx + 4..].join(" ")
            } else {
                String::new()
            };
            services.push(FailedServiceInfo {
                unit,
                load,
                active,
                sub,
                description,
            });
        }
    }
    services
}

/// 解析最近的内核 OOM 事件
pub fn parse_oom_alert(content: &str) -> Option<OomAlert> {
    let text = if let Some((_, rest)) = content.split_once("__OOM__") {
        let end = rest.find("__").unwrap_or(rest.len());
        &rest[..end]
    } else if content.contains("__UPTIME__")
        || content.contains("__DF__")
        || content.contains("__NET__")
    {
        return None;
    } else {
        content
    };

    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if lower.contains("oom-killer")
            || lower.contains("out of memory")
            || lower.contains("invoked oom-killer")
        {
            let mut pid = 0;
            let mut process_name = String::from("未知进程");
            if let Some(idx) = trimmed.find("process ") {
                let rest = &trimmed[idx + 8..];
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if let Some(p_str) = parts.first() {
                    pid = p_str.parse::<u32>().unwrap_or(0);
                }
                if let Some(name_part) = parts.get(1) {
                    process_name = name_part
                        .trim_matches(|c| c == '(' || c == ')' || c == ',')
                        .to_string();
                }
            }
            return Some(OomAlert {
                process_name,
                pid,
                message: trimmed.to_string(),
            });
        }
    }
    None
}
