//! SFTP 双栏文件管理器、传输队列与权限/编辑模态弹窗模块。

mod file_ops;
mod local_pane_view;
mod modals;
mod nav_actions;
mod panel_view;
mod queue_actions;
mod queue_view;

use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::{Icon, VirtualListScrollHandle};
use gpui_kit::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use transom_core::{RemoteFile, TabId, TransferId, TransferTask};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalFileEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
}

pub(crate) struct SftpEditorModalState {
    pub(crate) tab_id: TabId,
    pub(crate) local_path: PathBuf,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    pub(crate) textarea: Entity<TextareaState>,
    pub(crate) error: Option<String>,
}

pub(crate) struct SftpChmodModalState {
    pub(crate) tab_id: TabId,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    pub(crate) permissions: u32,
    pub(crate) octal_input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SftpInputTarget {
    RemoteMkdir,
    RemoteRename { old_path: String },
    LocalMkdir,
    LocalRename { old_path: PathBuf },
}

pub(crate) struct SftpInputModalState {
    pub(crate) tab_id: TabId,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) target: SftpInputTarget,
    pub(crate) error: Option<String>,
}

pub(crate) struct SftpPanelState {
    pub(crate) path: String,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) name_input: Entity<InputState>,
    pub(crate) remote_filter_input: Entity<InputState>,
    pub(crate) entries: Vec<RemoteFile>,
    pub(crate) selected: Option<String>,
    pub(crate) local_path: PathBuf,
    pub(crate) local_path_input: Entity<InputState>,
    pub(crate) local_filter_input: Entity<InputState>,
    pub(crate) local_entries: Vec<LocalFileEntry>,
    pub(crate) local_selected: Option<PathBuf>,
    pub(crate) local_error: Option<String>,
    pub(crate) local_scroll_handle: VirtualListScrollHandle,
    pub(crate) remote_scroll_handle: VirtualListScrollHandle,
    pub(crate) show_filter: bool,
    pub(crate) show_hidden: bool,
    pub(crate) sync_navigation: bool,
    pub(crate) queue: Vec<TransferTask>,
    pub(crate) resume_enabled: bool,
    pub(crate) queue_expanded: bool,
    pub(crate) queue_height: f32,
    pub(crate) drag_start_y: Option<f32>,
    pub(crate) drag_start_height: f32,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) active_op: Option<TransferId>,
    pub(crate) progress: Option<(u64, Option<u64>)>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    pub(crate) pending_edit_ops: HashMap<TransferId, (PathBuf, String, String)>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

pub(crate) fn local_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn list_local_dir(dir: &Path) -> Result<Vec<LocalFileEntry>, String> {
    let read_dir = std::fs::read_dir(dir).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let size = if is_dir {
            0
        } else {
            entry.metadata().map(|m| m.len()).unwrap_or(0)
        };
        entries.push(LocalFileEntry {
            name,
            path,
            is_dir,
            size,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    Ok(entries)
}

pub(crate) fn join_remote(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub(crate) fn parent_remote(path: &str) -> String {
    if path == "/" || path.is_empty() {
        return "/".to_string();
    }
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
        None => ".".to_string(),
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn sftp_file_icon(is_dir: bool, p: &Palette) -> AnyElement {
    if is_dir {
        Icon::new(IconName::Folder)
            .text_color(p.sidebar_accent_foreground)
            .into_any_element()
    } else {
        Icon::new(IconName::File)
            .text_color(p.muted_foreground)
            .into_any_element()
    }
}

/// 从终端标题或状态文本中尝试解析当前工作目录
pub(crate) fn extract_candidate_path(title: &str) -> Option<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if let Some((_, right)) = trimmed.split_once(':') {
        right.trim()
    } else {
        trimmed
    };
    let candidate = if let Some((left, _)) = candidate.split_once('(') {
        left.trim()
    } else {
        candidate
    };
    if candidate == "~" {
        Some(".".to_string())
    } else if let Some(rest) = candidate.strip_prefix("~/") {
        Some(format!("./{rest}"))
    } else if candidate.starts_with('/') || candidate.starts_with("./") {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// 大小写不敏感文件名模糊匹配（优先包含子串，次选按字符顺序子序列匹配）
pub(crate) fn fuzzy_match_name(name: &str, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }
    let name_lower = name.to_ascii_lowercase();
    if name_lower.contains(query_lower) {
        return true;
    }
    let mut name_chars = name_lower.chars();
    for q_ch in query_lower.chars() {
        if !name_chars.any(|n_ch| n_ch == q_ch) {
            return false;
        }
    }
    true
}

/// 异步调用操作系统原生通知中心弹出桌面通知，避免阻塞 UI 渲染主线程
pub(crate) fn send_desktop_notification(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    let _ = std::thread::Builder::new()
        .name("transom-desktop-notify".into())
        .spawn(move || {
            let _ = notify_rust::Notification::new()
                .appname("Transom")
                .summary(&summary)
                .body(&body)
                .show();
        });
}
