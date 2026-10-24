//! SFTP 双栏文件管理器、传输队列、内置编辑器悬浮面板与权限/属性模态弹窗模块。

mod chmod;
mod editor_ops;
mod editor_panel;
mod editor_view;
mod file_ops;
mod filter;
mod local_only;
mod local_pane_view;
mod modals;
mod nav_actions;
mod panel_view;
mod preview;
mod preview_actions;
mod preview_view;
mod properties;
mod queue_actions;
mod queue_view;
mod quick_paths;
mod status;
mod sync_actions;
mod sync_modal;

pub(crate) use self::editor_panel::EditorPanelState;
pub(crate) use self::preview::SftpPreviewResponse;
use self::preview::SftpPreviewState;
pub(crate) use self::quick_paths::LocalQuickPaths;
use crate::palette::Palette;
use chrono::{Local, TimeZone};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::{Icon, VirtualListScrollHandle};
use gpui_kit::*;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use transom_core::{
    ConnectionId, DirDiffEntry, RemoteFile, RemoteMetadata, TabId, TransferId, TransferTask,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalFileEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
}

pub(crate) struct SftpChmodModalState {
    pub(crate) tab_id: TabId,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    pub(crate) permissions: u32,
    pub(crate) octal_input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

/// 传输排除规则编辑弹窗状态。
pub(crate) struct SftpFilterModalState {
    pub(crate) textarea: Entity<TextareaState>,
    pub(crate) error: Option<String>,
}

/// 目录对比与增量同步弹窗状态。
pub(crate) struct SftpSyncModalState {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    /// 正在进行的对比操作 ID；对比结果异步回传后用于匹配。
    pub(crate) op_id: TransferId,
    pub(crate) local_root: PathBuf,
    pub(crate) remote_root: String,
    pub(crate) scanning: bool,
    pub(crate) entries: Vec<DirDiffEntry>,
    /// 与 `entries` 等长，`true` 表示该差异项参与同步。
    pub(crate) selected: Vec<bool>,
    pub(crate) skipped: u64,
    pub(crate) error: Option<String>,
    pub(crate) scroll_handle: VirtualListScrollHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SftpInputTarget {
    RemoteMkdir,
    RemoteRename { old_path: String },
    RemoteSymlink { target: String },
    LocalMkdir,
    LocalRename { old_path: PathBuf },
    LocalSymlink { target_path: PathBuf },
}

pub(crate) struct SftpInputModalState {
    pub(crate) tab_id: TabId,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) target: SftpInputTarget,
    pub(crate) error: Option<String>,
}

/// SFTP 双栏中的活动栏位：最近一次被鼠标点击的栏。
///
/// 设计原理（Why）：双栏共用同一键盘上下文（`SftpPanel`），Delete 等快捷键
/// 需要一个明确的目标栏位来源；以“最近点击”作为意图表达，符合文件管理器惯例，
/// 且无需引入额外的焦点注册机制。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SftpActivePane {
    #[default]
    Remote,
    Local,
}

pub(crate) struct SftpPanelState {
    pub(crate) path: String,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) name_input: Entity<InputState>,
    pub(crate) remote_filter_input: Entity<InputState>,
    pub(crate) entries: Vec<RemoteFile>,
    pub(crate) selected: Option<String>,
    /// 远端栏多选集合；`selected` 始终指向其中最近操作的主项。
    pub(crate) remote_selection: BTreeSet<String>,
    /// 远端 Shift 连选的锚点路径。
    pub(crate) remote_anchor: Option<String>,
    pub(crate) local_path: PathBuf,
    pub(crate) local_path_input: Entity<InputState>,
    pub(crate) local_filter_input: Entity<InputState>,
    pub(crate) local_entries: Vec<LocalFileEntry>,
    /// 打开面板时解析一次的常用目录，避免逐帧读取 XDG 配置。
    pub(crate) local_quick_paths: LocalQuickPaths,
    /// 本地终端专用：只渲染本地栏，不包含远端、队列与上传入口。
    pub(crate) local_only: bool,
    pub(crate) local_selected: Option<PathBuf>,
    /// 本地栏多选集合；`local_selected` 始终指向其中最近操作的主项。
    pub(crate) local_selection: BTreeSet<PathBuf>,
    /// Shift 连选的锚点路径。
    pub(crate) local_anchor: Option<PathBuf>,
    pub(crate) local_error: Option<String>,
    pub(crate) local_scroll_handle: VirtualListScrollHandle,
    pub(crate) remote_scroll_handle: VirtualListScrollHandle,
    pub(crate) show_filter: bool,
    pub(crate) show_hidden: bool,
    pub(crate) sync_navigation: bool,
    pub(crate) active_pane: SftpActivePane,
    pub(crate) queue: Vec<TransferTask>,
    pub(crate) resume_enabled: bool,
    pub(crate) queue_expanded: bool,
    pub(crate) queue_height: f32,
    pub(crate) drag_start_y: Option<f32>,
    pub(crate) drag_start_height: f32,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) active_op: Option<TransferId>,
    /// 当前正在由队列执行的任务集合，支持并行传输。
    pub(crate) active_transfers: HashSet<TransferId>,
    pub(crate) progress: Option<(u64, Option<u64>)>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    /// 瞬时状态消息的过期任务；替换消息时旧任务被取消。
    pub(super) status_task: Option<Task<()>>,
    /// 当前悬停预览；同一时刻只保留一个目标，切换目标或离开行时整体释放。
    pub(crate) hover_preview: Option<SftpPreviewState>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl SftpPanelState {
    /// 选中远程条目并标记远程栏为活动栏（行点击与右键菜单共用）。
    ///
    /// 设计原理（Why）：Delete 等快捷键按“活动栏”定位目标，若各处分散直接写
    /// `selected` 字段，右键操作另一栏后会残留旧活动栏，导致快捷键删错栏位；
    /// 统一经本方法赋值可从机制上杜绝该失配。
    pub(crate) fn select_remote(&mut self, path: String) {
        self.selected = Some(path.clone());
        self.remote_anchor = Some(path.clone());
        self.remote_selection.clear();
        self.remote_selection.insert(path);
        self.active_pane = SftpActivePane::Remote;
    }

    /// 设置远端主项但保留已有多选集合，供右键菜单定位单项操作。
    pub(crate) fn set_remote_primary(&mut self, path: String) {
        self.selected = Some(path.clone());
        self.remote_selection.insert(path);
        self.active_pane = SftpActivePane::Remote;
    }

    /// Ctrl/Cmd 点击切换远端条目的选中状态。
    pub(crate) fn toggle_remote_selection(&mut self, path: String) {
        if self.remote_selection.remove(&path) {
            if self.selected.as_deref() == Some(path.as_str()) {
                self.selected = self.remote_selection.iter().next_back().cloned();
            }
        } else {
            self.remote_selection.insert(path.clone());
            self.selected = Some(path.clone());
        }
        self.remote_anchor = Some(path);
        self.active_pane = SftpActivePane::Remote;
    }

    /// Shift 连选：用可见列表中的一段路径替换远端集合。
    pub(crate) fn select_remote_range(
        &mut self,
        paths: impl IntoIterator<Item = String>,
        primary: String,
    ) {
        self.remote_selection = paths.into_iter().collect();
        self.remote_selection.insert(primary.clone());
        self.selected = Some(primary);
        self.active_pane = SftpActivePane::Remote;
    }

    /// 返回当前需要批量操作的远端路径；无多选时退化为单选主项。
    pub(crate) fn selected_remote_paths(&self) -> Vec<String> {
        if self.remote_selection.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.remote_selection.iter().cloned().collect()
        }
    }

    pub(crate) fn clear_remote_selection(&mut self) {
        self.selected = None;
        self.remote_selection.clear();
        self.remote_anchor = None;
    }

    /// 刷新远端列表后剔除已不存在的选中项，并修正主项与锚点。
    pub(crate) fn sync_remote_selection(&mut self, valid: &BTreeSet<String>) {
        self.remote_selection.retain(|path| valid.contains(path));
        if self
            .selected
            .as_ref()
            .is_some_and(|path| !valid.contains(path))
        {
            self.selected = self.remote_selection.iter().next_back().cloned();
        }
        if self
            .remote_anchor
            .as_ref()
            .is_some_and(|path| !valid.contains(path))
        {
            self.remote_anchor = self.selected.clone();
        }
        if self.remote_selection.is_empty() {
            self.selected = None;
            self.remote_anchor = None;
        }
    }

    /// 单选一个本地条目，同时清空多选集合并重置 Shift 锚点。
    pub(crate) fn select_local(&mut self, path: PathBuf) {
        self.local_selected = Some(path.clone());
        self.local_anchor = Some(path.clone());
        self.local_selection.clear();
        self.local_selection.insert(path);
        self.active_pane = SftpActivePane::Local;
    }

    /// 设置本地主项但保留已有多选集合，供右键菜单定位单项操作。
    pub(crate) fn set_local_primary(&mut self, path: PathBuf) {
        self.local_selected = Some(path.clone());
        self.local_selection.insert(path);
        self.active_pane = SftpActivePane::Local;
    }

    /// Ctrl/Cmd 点击切换单个条目的选中状态。
    pub(crate) fn toggle_local_selection(&mut self, path: PathBuf) {
        if self.local_selection.remove(&path) {
            if self.local_selected.as_ref() == Some(&path) {
                self.local_selected = self.local_selection.iter().next_back().cloned();
            }
        } else {
            self.local_selection.insert(path.clone());
            self.local_selected = Some(path.clone());
        }
        self.local_anchor = Some(path);
        self.active_pane = SftpActivePane::Local;
    }

    /// Shift 连选：用可见列表中的一段路径替换当前集合。
    pub(crate) fn select_local_range(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        primary: PathBuf,
    ) {
        self.local_selection = paths.into_iter().collect();
        self.local_selection.insert(primary.clone());
        self.local_selected = Some(primary);
        self.active_pane = SftpActivePane::Local;
    }

    /// 返回当前需要批量操作的本地路径；无多选时退化为单选主项。
    pub(crate) fn selected_local_paths(&self) -> Vec<PathBuf> {
        if self.local_selection.is_empty() {
            self.local_selected.iter().cloned().collect()
        } else {
            self.local_selection.iter().cloned().collect()
        }
    }

    pub(crate) fn clear_local_selection(&mut self) {
        self.local_selected = None;
        self.local_selection.clear();
        self.local_anchor = None;
    }

    /// 刷新目录后剔除已不存在的选中项，并修正主项与锚点。
    pub(crate) fn sync_local_selection(&mut self, valid: &BTreeSet<PathBuf>) {
        self.local_selection.retain(|path| valid.contains(path));
        if self
            .local_selected
            .as_ref()
            .is_some_and(|path| !valid.contains(path))
        {
            self.local_selected = self.local_selection.iter().next_back().cloned();
        }
        if self
            .local_anchor
            .as_ref()
            .is_some_and(|path| !valid.contains(path))
        {
            self.local_anchor = self.local_selected.clone();
        }
        if self.local_selection.is_empty() {
            self.local_selected = None;
            self.local_anchor = None;
        }
    }

    /// 汇总所有活动任务的字节进度；无活动任务时清空进度。
    pub(crate) fn recompute_progress(&mut self) {
        if self.active_transfers.is_empty() {
            self.progress = None;
            return;
        }
        let mut transferred = 0u64;
        let mut total = 0u64;
        let mut total_known = true;
        for task in &self.queue {
            if !self.active_transfers.contains(&task.id) {
                continue;
            }
            transferred = transferred.saturating_add(task.transferred);
            match task.size {
                Some(size) => total = total.saturating_add(size),
                None => total_known = false,
            }
        }
        self.progress = Some((transferred, total_known.then_some(total)));
    }
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

/// 通用文件属性快照（远端与本地共用）。
///
/// 设计原理（Why）：
/// - 实现初衷：远端与本地属性来源不同（SFTP lstat / 本地 lstat），但展示结构一致，
///   统一为同一快照类型可让弹窗渲染逻辑只维护一份；
/// - 核心优势：所有差异化字段均为 `Option`，缺失时展示层跳过对应行，
///   不用为不同来源编写分支渲染；
/// - 代价与局限：快照为纯展示数据，不保留原始元数据，如需二次操作（如 chmod）
///   必须重新读取。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileProperties {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: String,
    pub(crate) size: Option<u64>,
    pub(crate) permissions: Option<u32>,
    pub(crate) owner: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) modified: Option<u32>,
    pub(crate) accessed: Option<u32>,
    pub(crate) link_target: Option<String>,
}

/// 文件属性查看弹窗状态（只读展示，无输入控件）。
pub(crate) struct SftpPropertiesModalState {
    /// 属性来源描述（「远端」/「本地」），用于弹窗副标题
    pub(crate) scope: String,
    pub(crate) properties: FileProperties,
}

/// 由远端属性契约转换为展示层属性快照
pub(crate) fn remote_file_properties(metadata: &RemoteMetadata) -> FileProperties {
    let name = metadata
        .path
        .trim_end_matches('/')
        .rsplit_once('/')
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| metadata.path.clone());
    FileProperties {
        name,
        path: metadata.path.clone(),
        kind: file_kind_text(metadata.is_symlink, metadata.is_dir).to_string(),
        size: if metadata.is_dir {
            None
        } else {
            Some(metadata.size)
        },
        permissions: metadata.permissions,
        owner: identity_text(metadata.user.as_deref(), metadata.uid),
        group: identity_text(metadata.group.as_deref(), metadata.gid),
        modified: metadata.mtime,
        accessed: metadata.atime,
        link_target: metadata.link_target.clone(),
    }
}

/// 读取本地文件系统属性快照（lstat 语义，不跟随符号链接）
pub(crate) fn local_file_properties(path: &Path) -> Result<FileProperties, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let is_symlink = metadata.file_type().is_symlink();
    let link_target = if is_symlink {
        std::fs::read_link(path)
            .ok()
            .map(|target| target.display().to_string())
    } else {
        None
    };
    #[cfg(unix)]
    let (owner, group) = {
        use std::os::unix::fs::MetadataExt;
        (
            identity_text(
                local_user_name(metadata.uid()).as_deref(),
                Some(metadata.uid()),
            ),
            identity_text(
                local_group_name(metadata.gid()).as_deref(),
                Some(metadata.gid()),
            ),
        )
    };
    #[cfg(not(unix))]
    let (owner, group): (Option<String>, Option<String>) = (None, None);
    Ok(FileProperties {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
        path: path.display().to_string(),
        kind: file_kind_text(is_symlink, metadata.is_dir()).to_string(),
        size: if metadata.is_dir() {
            None
        } else {
            Some(metadata.len())
        },
        permissions: local_permissions(&metadata),
        owner,
        group,
        modified: unix_seconds(metadata.modified().ok()),
        accessed: unix_seconds(metadata.accessed().ok()),
        link_target,
    })
}

/// 文件类型的中文描述（符号链接优先于目录/文件判定）
fn file_kind_text(is_symlink: bool, is_dir: bool) -> &'static str {
    if is_symlink {
        "符号链接"
    } else if is_dir {
        "文件夹"
    } else {
        "文件"
    }
}

/// 拼接身份展示文本：名称与数字 ID 同时可用时一并展示，只有其一则单独展示
fn identity_text(name: Option<&str>, id: Option<u32>) -> Option<String> {
    match (name, id) {
        (Some(name), Some(id)) => Some(format!("{name} ({id})")),
        (Some(name), None) => Some(name.to_string()),
        (None, Some(id)) => Some(id.to_string()),
        (None, None) => None,
    }
}

/// 将 Unix 秒级时间戳格式化为本地时区 `YYYY-MM-DD HH:MM:SS`。
///
/// 设计原理（Why）：属性弹窗的时间需要与用户在服务器上 `ls -l` 看到的
/// 本地时间对齐（而非 UTC），故借助已在依赖树内的 chrono 做本地时区转换。
pub(crate) fn format_unix_time(seconds: u32) -> String {
    match Local.timestamp_opt(i64::from(seconds), 0).single() {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{seconds} (Unix 时间戳)"),
    }
}

/// 将系统时间转换为 Unix 秒（超出 u32 可表示范围时饱和截断）
fn unix_seconds(time: Option<std::time::SystemTime>) -> Option<u32> {
    let seconds = time?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.min(u64::from(u32::MAX)) as u32)
}

#[cfg(unix)]
fn local_permissions(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn local_permissions(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

/// 从 `/etc/passwd` 或 `/etc/group` 内容中解析指定 ID 对应的名称。
///
/// 设计原理（Why）：本地属性展示需要把 uid/gid 映射为可读名称；在不引入
/// libc 依赖的前提下，直接解析系统账号文件是最轻量且可单测验证的方式。
fn parse_id_name(content: &str, id: u32) -> Option<String> {
    for line in content.lines() {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password_placeholder = fields.next();
        if fields.next().and_then(|field| field.parse::<u32>().ok()) == Some(id) {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(unix)]
fn local_user_name(uid: u32) -> Option<String> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    parse_id_name(&content, uid)
}

#[cfg(unix)]
fn local_group_name(gid: u32) -> Option<String> {
    let content = std::fs::read_to_string("/etc/group").ok()?;
    parse_id_name(&content, gid)
}

#[cfg(test)]
mod tests {
    // 显式导入被测助手函数：避免 `use super::*` 引入 gpui 同名 `test` 属性宏
    // 导致 `#[test]` 展开递归（recursion limit reached）
    use super::{
        file_kind_text, format_unix_time, identity_text, parse_id_name, remote_file_properties,
    };
    use transom_core::RemoteMetadata;

    #[test]
    fn test_parse_id_name() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nalice:x:1000:1000::/home/alice:/bin/sh\n";
        assert_eq!(parse_id_name(passwd, 0).as_deref(), Some("root"));
        assert_eq!(parse_id_name(passwd, 1000).as_deref(), Some("alice"));
        assert_eq!(parse_id_name(passwd, 42), None);
        assert_eq!(parse_id_name("", 0), None);
        assert_eq!(parse_id_name("broken-line", 0), None);
    }

    #[test]
    fn test_identity_text() {
        assert_eq!(
            identity_text(Some("root"), Some(0)).as_deref(),
            Some("root (0)")
        );
        assert_eq!(identity_text(Some("root"), None).as_deref(), Some("root"));
        assert_eq!(identity_text(None, Some(1000)).as_deref(), Some("1000"));
        assert_eq!(identity_text(None, None), None);
    }

    #[test]
    fn test_file_kind_text() {
        assert_eq!(file_kind_text(true, true), "符号链接");
        assert_eq!(file_kind_text(true, false), "符号链接");
        assert_eq!(file_kind_text(false, true), "文件夹");
        assert_eq!(file_kind_text(false, false), "文件");
    }

    #[test]
    fn test_format_unix_time_shape() {
        let text = format_unix_time(0);
        assert_eq!(text.len(), 19);
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[7..8], "-");
        assert_eq!(&text[10..11], " ");
        assert_eq!(&text[13..14], ":");
        assert_eq!(&text[16..17], ":");
    }

    #[test]
    fn test_remote_file_properties_name_and_size() {
        let metadata = RemoteMetadata {
            path: "/var/log/app.log".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 2048,
            uid: Some(0),
            user: Some("root".to_string()),
            gid: Some(0),
            group: None,
            permissions: Some(0o100644),
            atime: Some(1),
            mtime: Some(2),
            link_target: None,
        };
        let properties = remote_file_properties(&metadata);
        assert_eq!(properties.name, "app.log");
        assert_eq!(properties.kind, "文件");
        assert_eq!(properties.size, Some(2048));
        assert_eq!(properties.owner.as_deref(), Some("root (0)"));
        assert_eq!(properties.group.as_deref(), Some("0"));
    }
}
