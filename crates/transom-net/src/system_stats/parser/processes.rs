//! 进程列表、监听端口、失败服务与 OOM 事件解析。

use std::collections::HashSet;

use transom_core::{FailedServiceInfo, OomAlert, PortListeningInfo, ProcessInfo};

/// 解析进程采样输出，并按 TGID 将同一进程的线程归并成一行。
///
/// 新格式由 `ps -eo pid,tgid,ppid,user,%cpu,%mem,rss,comm,args` 产生；
/// 旧格式缺少 TGID 时以 PID 作为进程标识，保持对旧采样命令的兼容。
/// 同一进程的主线程记录优先作为进程行，避免把线程名和重复 RSS 展示成进程。
pub fn parse_processes(content: &str) -> Vec<ProcessInfo> {
    let header = content.lines().next().unwrap_or_default();
    let header_columns: Vec<&str> = header.split_whitespace().collect();
    let has_tgid = header_columns
        .iter()
        .any(|column| column.eq_ignore_ascii_case("TGID"));
    let command_columns = header_columns
        .iter()
        .filter(|column| column.eq_ignore_ascii_case("COMMAND"))
        .count();
    let has_args = header_columns
        .iter()
        .any(|column| column.eq_ignore_ascii_case("ARGS"))
        || command_columns > 1;
    let (ppid_idx, user_idx, cpu_idx, mem_idx, rss_idx, command_idx): (
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) = if has_tgid {
        (2, 3, 4, 5, 6, 7)
    } else {
        (1, 2, 3, 4, 5, 6)
    };
    let args_idx = command_idx.saturating_add(1);
    let mut rows: Vec<(u32, ProcessInfo)> = Vec::with_capacity(50);

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() <= command_idx {
            continue;
        }
        let Ok(pid) = parts[0].parse::<u32>() else {
            continue;
        };
        let tgid = if has_tgid {
            let Ok(tgid) = parts[1].parse::<u32>() else {
                continue;
            };
            tgid
        } else {
            pid
        };
        let Ok(ppid) = parts[ppid_idx].parse::<u32>() else {
            continue;
        };
        let Ok(cpu_percent) = parts[cpu_idx].parse::<f32>() else {
            continue;
        };
        let Ok(mem_percent) = parts[mem_idx].parse::<f32>() else {
            continue;
        };
        let Ok(rss_kb) = parts[rss_idx].parse::<u64>() else {
            continue;
        };

        let (cmdline, args) = if has_args {
            let cmdline = parts[command_idx].to_string();
            let args = if parts.len() > args_idx {
                Some(parts[args_idx..].join(" "))
            } else {
                None
            };
            (cmdline, args)
        } else {
            let full = parts[command_idx..].join(" ");
            (full, None)
        };

        rows.push((
            tgid,
            ProcessInfo {
                pid,
                ppid,
                user: parts[user_idx].to_string(),
                cpu_percent,
                mem_percent,
                rss_bytes: rss_kb.saturating_mul(1024),
                cmdline,
                args,
            },
        ));
    }

    // ps 的 %CPU 和 RSS 是进程级统计，线程行会重复展示相同数值；因此每组只
    // 保留主线程行，绝不把线程指标相加，否则会造成 CPU 与内存双重计算。
    rows.sort_unstable_by(|(left_tgid, left), (right_tgid, right)| {
        left_tgid.cmp(right_tgid).then_with(|| {
            let left_is_leader = left.pid == *left_tgid;
            let right_is_leader = right.pid == *right_tgid;
            right_is_leader
                .cmp(&left_is_leader)
                .then_with(|| left.pid.cmp(&right.pid))
        })
    });

    let mut processes = Vec::with_capacity(rows.len());
    for (tgid, mut process) in rows {
        if processes
            .last()
            .is_some_and(|previous: &ProcessInfo| previous.pid == tgid)
        {
            continue;
        }
        process.pid = tgid;
        processes.push(process);
    }
    processes
}

/// 统计唯一 TGID 数量；旧格式仅含 PID 时同样按唯一值计数。
///
/// 折叠态只需要总数，用仅含 pid 的窄列输出把每轮传输量从全量明细的数十 KB
/// 压到数 KB，代价是计数与明细采样发生在不同时刻，两者可能相差一两个进程。
pub fn parse_process_count(content: &str) -> u32 {
    let mut process_ids = HashSet::with_capacity(content.lines().count());
    for line in content.lines() {
        if let Ok(process_id) = line.trim().parse::<u32>() {
            process_ids.insert(process_id);
        }
    }
    u32::try_from(process_ids.len()).unwrap_or(u32::MAX)
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
