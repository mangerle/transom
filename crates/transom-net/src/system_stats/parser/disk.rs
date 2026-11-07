//! 磁盘容量与文件系统挂载点解析。

use transom_core::FilesystemUsage;

/// 从 df -P -k / 解析根目录磁盘使用情况：(used_bytes, total_bytes)
pub fn parse_disk(content: &str) -> (u64, u64) {
    let text = if let Some((_, rest)) = content.split_once("__DF__") {
        rest
    } else {
        content
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("__") {
            break;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // df -P 格式下一行至少 6 项：Filesystem 1024-blocks Used Available Capacity Mounted on
        if parts.len() >= 6 && parts.last() == Some(&"/") {
            let total_idx = parts.len() - 5;
            let used_idx = parts.len() - 4;
            if let (Ok(total_kb), Ok(used_kb)) = (
                parts[total_idx].parse::<u64>(),
                parts[used_idx].parse::<u64>(),
            ) {
                return (used_kb * 1024, total_kb * 1024);
            }
        }
    }

    // 容错兜底：如果未显式挂载在 "/"，寻找带百分比的记录行
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("__") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 6 && parts[parts.len() - 2].ends_with('%') {
            let total_idx = parts.len() - 5;
            let used_idx = parts.len() - 4;
            if let (Ok(total_kb), Ok(used_kb)) = (
                parts[total_idx].parse::<u64>(),
                parts[used_idx].parse::<u64>(),
            ) {
                return (used_kb * 1024, total_kb * 1024);
            }
        }
    }

    (0, 0)
}

/// 解析 `df -P -k` 的 `__DF__` 段，返回所有真实挂载点（最多 12 个）。
///
/// 只保留以 `/` 开头的挂载点：`df` 也可能列出伪文件系统，界面上的磁盘
/// 看板只关心真实路径；同一挂载点重复出现时保留首行。
pub fn parse_filesystems(content: &str) -> Vec<FilesystemUsage> {
    let text = if let Some((_, rest)) = content.split_once("__DF__") {
        rest
    } else {
        content
    };
    let mut filesystems: Vec<FilesystemUsage> = Vec::with_capacity(8);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("__") {
            if trimmed.starts_with("__") && !filesystems.is_empty() {
                break;
            }
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // Filesystem 1024-blocks Used Available Capacity Mounted-on
        if parts.len() < 6 {
            continue;
        }
        let Some(mount) = parts.last() else {
            continue;
        };
        if !mount.starts_with('/') {
            continue;
        }
        let total_idx = parts.len() - 5;
        let used_idx = parts.len() - 4;
        let (Ok(total_kb), Ok(used_kb)) = (
            parts[total_idx].parse::<u64>(),
            parts[used_idx].parse::<u64>(),
        ) else {
            continue;
        };
        if total_kb == 0 {
            continue;
        }
        if filesystems.iter().any(|entry| entry.mount == *mount) {
            continue;
        }
        filesystems.push(FilesystemUsage {
            mount: (*mount).to_string(),
            used_bytes: used_kb.saturating_mul(1024),
            total_bytes: total_kb.saturating_mul(1024),
        });
        if filesystems.len() >= 12 {
            break;
        }
    }
    filesystems
}
