//! SFTP 双栏文件管理器、传输队列、内置编辑器悬浮面板与权限/属性模态弹窗模块。

mod chmod;
mod columns;
mod drop_upload;
mod file_ops;
mod file_row;
mod file_row_content;
mod filter;
mod input_modal;
mod local_async;
mod local_listing;
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
mod remote_actions;
mod remote_content;
mod remote_nav;
mod remote_pane;
mod remote_pane_state;
mod remote_pane_toolbar;
mod row_interaction;
mod search_modal;
mod selection;
mod side_remote;
mod side_remote_view;
mod state;
mod status_message;
mod sync_actions;
mod sync_modal;
mod task;
mod transfer_filter_modal;
mod transfer_presentation;

pub(crate) use self::file_row_content::{FileRowContentOptions, render_file_row_content};
pub(crate) use self::input_modal::SftpInputModalParams;
pub(crate) use self::local_async::spawn_blocking_local;
pub(crate) use self::local_listing::LocalDirectoryLister;
pub(crate) use self::modals::{
    EditorGotoLineModalState, SftpConfirmModalState, SftpModals, SftpPendingDelete,
};
pub(crate) use self::overlay::DropOverlayState;
pub(crate) use self::preview::SftpPreviewResponse;
pub(crate) use self::properties::{
    SftpPropertiesModalState, format_unix_time, remote_file_properties,
};
pub(crate) use self::quick_paths::LocalQuickPaths;
pub(crate) use self::search_modal::SftpSearchModalState;
pub(crate) use self::side_remote::SftpActivePane;
pub(crate) use self::state::{
    LocalFileEntry, RemoteDirCacheEntry, SFTP_DIR_CACHE_TTL, SftpChmodModalState,
    SftpFilterModalState, SftpInputModalState, SftpInputTarget, SftpLocalPaneInit,
    SftpLocalPaneState, SftpPanelState, SftpQueueState, SftpRemotePaneRef, SftpRemotePaneState,
    SftpSyncModalState,
};
pub(crate) use self::task::{
    SftpTransferRequest, TransferKind, TransferMode, TransferStatus, TransferTask,
};
pub(crate) use crate::editor::{EditorDragTracker, PendingEditorClose};
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
    LocalDirectoryLister::list_sync(dir).map_err(|error| error.to_string())
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

/// 把 OSC 7 上报的路径规范成 SFTP 远端面板可用的目录。
///
/// 设计原理（Why）：OSC 7 的标准载荷是以 `/` 开头的绝对 POSIX 路径；`~`、
/// 相对路径、Windows 盘符都不是 SFTP 远端列目录能直接消化的形态，一律丢弃。
/// 控制字符在终端解析层已经拒绝，这里再挡一次，避免其他调用方绕过解析器。
/// 远端路径不做存在性检查：目录不存在时由列表请求返回错误，沿用现有错误通道。
pub(crate) fn normalize_remote_cwd(path: &str) -> Option<String> {
    if !path.starts_with('/') || path.chars().any(char::is_control) {
        return None;
    }
    Some(path.to_string())
}

/// 把 OSC 7 上报的路径规范成本地文件面板可用的目录。
///
/// 只有真实存在且是目录时才返回，避免把命令名或无效文本当成路径；
/// 只接受绝对路径，防止相对路径被误解析到进程当前目录。Windows 的 OSC 7
/// 常写成 `/C:/Users/...`，去掉前导斜杠后再试一次。
pub(crate) fn normalize_local_cwd(path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let absolute = path.starts_with('/') || (cfg!(windows) && has_drive_prefix(path));
    if !absolute {
        return None;
    }
    let direct = PathBuf::from(path);
    if direct.is_dir() {
        return Some(direct);
    }
    #[cfg(windows)]
    if let Some(rest) = path.strip_prefix('/') {
        let stripped = PathBuf::from(rest);
        if stripped.is_dir() {
            return Some(stripped);
        }
    }
    None
}

/// 是否为 `C:/...` 或 `C:\...` 形式的 Windows 盘符路径。
#[cfg(windows)]
fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

/// 非 Windows 平台不存在盘符路径。
#[cfg(not(windows))]
fn has_drive_prefix(_path: &str) -> bool {
    false
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
    let spawn = std::thread::Builder::new()
        .name("transom-desktop-notify".into())
        .spawn(move || {
            // GNOME 会在"通知来自当前聚焦窗口的进程"时立刻以 reason=2 关闭通知；
            // 外部 notify-send 进程没有窗口，行为与终端里手敲 notify-send 一致。
            // 优先走它，命令缺失或失败再回退跨平台的 notify-rust。
            #[cfg(target_os = "linux")]
            if notify_via_send(&summary, &body) {
                log::debug!("桌面通知已通过 notify-send 发送");
                return;
            }
            if let Err(error) = notify_rust::Notification::new()
                .appname("Transom")
                .summary(&summary)
                .body(&body)
                .show()
            {
                log::warn!("发送桌面通知失败，摘要: {summary}，原因: {error}");
            }
        });
    if let Err(error) = spawn {
        log::warn!("创建桌面通知线程失败，原因: {error}");
    }
}

/// 用系统 `notify-send` 发送通知；命令不存在、超时或返回非零时返回 `false`。
#[cfg(target_os = "linux")]
fn notify_via_send(summary: &str, body: &str) -> bool {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let Ok(mut child) = Command::new("notify-send")
        .arg("--app-name=Transom")
        .arg("--urgency=normal")
        .arg(summary)
        .arg(body)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_local_cwd, normalize_remote_cwd};

    #[test]
    fn normalize_remote_cwd_accepts_only_absolute_posix_paths() {
        assert_eq!(
            normalize_remote_cwd("/var/log"),
            Some("/var/log".to_string())
        );
        assert_eq!(normalize_remote_cwd("/"), Some("/".to_string()));
        assert_eq!(
            normalize_remote_cwd("/home/a b"),
            Some("/home/a b".to_string())
        );
    }

    #[test]
    fn normalize_remote_cwd_rejects_relative_and_control_input() {
        assert_eq!(normalize_remote_cwd(""), None);
        assert_eq!(normalize_remote_cwd("vim file.txt"), None);
        assert_eq!(normalize_remote_cwd("~"), None);
        assert_eq!(normalize_remote_cwd("~/nginx"), None);
        assert_eq!(normalize_remote_cwd("C:\\Users"), None);
        assert_eq!(normalize_remote_cwd("/var/\nlog"), None);
    }

    #[test]
    fn normalize_local_cwd_requires_existing_directory() {
        let temp = std::env::temp_dir();
        assert_eq!(normalize_local_cwd(&temp.display().to_string()), Some(temp));
        assert_eq!(normalize_local_cwd(""), None);
        assert_eq!(normalize_local_cwd("vim file.txt"), None);
        assert_eq!(normalize_local_cwd("."), None, "相对路径不能当 cwd");
        assert_eq!(normalize_local_cwd("/non_existent_dir_987654321"), None);
    }
}
