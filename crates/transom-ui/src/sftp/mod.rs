//! SFTP 双栏文件管理器、传输队列、内置编辑器悬浮面板与权限/属性模态弹窗模块。

mod chmod;
mod drop_upload;
mod file_ops;
mod filter;
mod local_async;
mod local_only;
mod local_pane_view;
mod modals;
mod nav_actions;
mod overlay;
mod panel_view;
mod preview;
mod preview_actions;
mod preview_view;
mod properties;
mod queue_actions;
mod queue_view;
mod quick_paths;
mod remote_content;
mod remote_pane;
mod side_remote;
mod side_remote_view;
mod state;
mod status;
mod sync_actions;
mod sync_modal;
mod vfs_entry_view;
mod vfs_provider;

pub(crate) use self::overlay::DropOverlayState;
pub(crate) use self::preview::SftpPreviewResponse;
pub(crate) use self::properties::{
    SftpPropertiesModalState, format_unix_time, remote_file_properties,
};
pub(crate) use self::quick_paths::LocalQuickPaths;
pub(crate) use self::side_remote::SftpActivePane;
pub(crate) use self::state::{
    LocalFileEntry, SftpChmodModalState, SftpFilterModalState, SftpInputModalState,
    SftpInputTarget, SftpPanelState, SftpSyncModalState,
};
pub(crate) use self::vfs_entry_view::{VfsRowRenderOptions, render_vfs_row_content};
pub(crate) use self::vfs_provider::LocalVfsProvider;
pub(crate) use crate::editor::{EditorDragTracker, EditorPanelState, PendingEditorClose};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;
use std::path::{Path, PathBuf};

pub(crate) fn local_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn list_local_dir(dir: &Path) -> Result<Vec<LocalFileEntry>, String> {
    let entries = LocalVfsProvider::list_sync(dir).map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .map(|entry| LocalFileEntry {
            name: entry.name,
            path: PathBuf::from(entry.path),
            is_dir: entry.is_dir,
            size: entry.size,
        })
        .collect())
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

pub(crate) use crate::format::format_size;

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
