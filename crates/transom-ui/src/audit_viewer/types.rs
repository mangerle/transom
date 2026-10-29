//! 审计日志查看器数据模型与文件扫描工具。

use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use std::io::Read;
use std::path::{Path, PathBuf};
use transom_platform::paths;

/// 单次读取最大字节数（2 MiB），防止大文件撑爆 UI 线程。
pub(crate) const MAX_AUDIT_LOG_BYTES: usize = 2 * 1024 * 1024;

/// 审计日志条目元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditLogEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub modified_display: String,
}

/// 审计日志查看器状态。
pub(crate) struct AuditViewer {
    pub target_title: String,
    pub session_name: Option<String>,
    pub host: Option<String>,
    pub entries: Vec<AuditLogEntry>,
    pub selected_index: usize,
    pub content: String,
    pub is_truncated: bool,
    pub search_input: Entity<InputState>,
    pub show_all: bool,
}

/// 扫描审计目录并按会话与过滤条件返回条目。
pub(crate) fn find_session_audit_logs(
    audit_dir: &Path,
    session_name: Option<&str>,
    host: Option<&str>,
    show_all: bool,
) -> Vec<AuditLogEntry> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(audit_dir) else {
        return entries;
    };

    let prefix = if !show_all && let (Some(name), Some(host)) = (session_name, host) {
        let label = paths::sanitize_audit_label(name);
        let host = paths::sanitize_audit_label(host);
        Some(format!("{label}-{host}"))
    } else {
        None
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("log") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();

        if let Some(prefix) = &prefix
            && !file_name.starts_with(prefix)
        {
            continue;
        }

        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let size_bytes = meta.len();
        let modified_display = meta
            .modified()
            .ok()
            .map(|time| {
                let dt: chrono::DateTime<chrono::Local> = time.into();
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_else(|| "-".to_string());

        entries.push(AuditLogEntry {
            path,
            file_name,
            size_bytes,
            modified_display,
        });
    }

    entries.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    entries
}

/// 从文件流式读取纯文本并剥离 ANSI 转义字符。
pub(crate) fn read_log_content(path: &Path, max_bytes: usize) -> (String, bool) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return (rust_i18n::t!("audit_viewer.open_failed").to_string(), false);
    };
    let mut buffer = Vec::new();
    let mut take = (&mut file).take((max_bytes + 1) as u64);
    if take.read_to_end(&mut buffer).is_err() {
        return (rust_i18n::t!("audit_viewer.read_failed").to_string(), false);
    }
    let is_truncated = buffer.len() > max_bytes;
    if is_truncated {
        buffer.truncate(max_bytes);
    }
    let raw = String::from_utf8_lossy(&buffer);
    (strip_ansi_escapes(&raw), is_truncated)
}

/// 轻量级剥离终端 ANSI 转义字符序列并保留规整纯文本。
pub(crate) fn strip_ansi_escapes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == 'm' || c == 'K' || c == 'H' || c == 'J' {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\x07' || c == '\x1b' {
                        break;
                    }
                }
            }
        } else if ch == '\r' {
            // 忽略回车符，保留换行
            if chars.peek() != Some(&'\n') {
                output.push('\n');
            }
        } else {
            output.push(ch);
        }
    }
    output
}

pub(crate) fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
