//! 模型层通用命名与时间工具。

use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn normalize_name(name: String, fallback: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.chars().take(super::MAX_NAME_CHARS).collect()
}

pub(super) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
