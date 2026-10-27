//! SFTP 双栏文件面板状态、选择状态机与各模态弹窗状态数据结构。

use super::overlay::DropOverlayState;
use super::preview::SftpPreviewState;
use super::quick_paths::LocalQuickPaths;
use super::side_remote::{SftpActivePane, SftpClipboardData, SftpSideRemoteState};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use transom_core::{ConnectionId, DirDiffEntry, RemoteFile, TabId, TransferId, TransferTask};

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
    /// 新建远端文件并直接进入编辑面板；提交后由编辑器打开空缓冲区。
    RemoteEditorNewFile,
    RemoteRename {
        old_path: String,
    },
    RemoteSymlink {
        target: String,
    },
    LocalMkdir,
    LocalRename {
        old_path: PathBuf,
    },
    LocalSymlink {
        target_path: PathBuf,
    },
}

pub(crate) struct SftpInputModalState {
    pub(crate) tab_id: TabId,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) target: SftpInputTarget,
    pub(crate) error: Option<String>,
}

/// SFTP 双栏中的活动栏位与主面板状态。
pub(crate) struct SftpPanelState {
    /// 面板列是否展开显示。
    ///
    /// 设计原理（Why）：`tab.sftp` 是 `Some` 只代表传输队列与目录状态存在，
    /// 不再等于"面板开着"——拖文件到终端上传要在不打开面板的前提下也持有队列，
    /// 因此建出状态、把进度改由终端右下角浮层呈现。凡是决定"要不要渲染面板列/
    /// 活动栏是否高亮"的地方一律读本字段，而不是读 `Option` 本身。
    pub(crate) open: bool,
    /// 终端右下角上传浮层的状态（面板未展开时用它承接进度可见性）。
    pub(crate) overlay: DropOverlayState,
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
    /// 本地栏是否折叠为竖条。
    pub(crate) local_collapsed: bool,
    /// 主远端栏是否折叠为竖条。
    pub(crate) remote_collapsed: bool,
    pub(crate) local_selected: Option<PathBuf>,
    /// 本地栏多选集合；`local_selected` 始终指向其中最近操作的主项。
    pub(crate) local_selection: BTreeSet<PathBuf>,
    /// Shift 连选的锚点路径。
    pub(crate) local_anchor: Option<PathBuf>,
    pub(crate) local_error: Option<String>,
    pub(crate) local_scroll_handle: VirtualListScrollHandle,
    pub(crate) remote_scroll_handle: VirtualListScrollHandle,
    /// 并列挂载的其他远端文件会话列表。
    pub(crate) side_remotes: Vec<SftpSideRemoteState>,
    /// 跨列文件剪贴板（Ctrl+C / Ctrl+V）。
    pub(crate) clipboard: Option<SftpClipboardData>,
    /// 挂载新远端浮层菜单是否打开。
    pub(crate) mount_menu_open: bool,
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

    /// 是否所有文件列均已处于折叠收起状态。
    pub(crate) fn is_all_collapsed(&self) -> bool {
        if !self.local_collapsed {
            return false;
        }
        if !self.local_only && !self.remote_collapsed {
            return false;
        }
        if self.side_remotes.iter().any(|s| !s.collapsed) {
            return false;
        }
        true
    }

    /// 统计当前折叠为竖条的列数（用于全收起时动态计算紧凑侧边栏宽度）。
    pub(crate) fn collapsed_columns_count(&self) -> usize {
        let mut count = 0;
        if self.local_collapsed {
            count += 1;
        }
        if !self.local_only && self.remote_collapsed {
            count += 1;
        }
        count += self.side_remotes.iter().filter(|s| s.collapsed).count();
        count
    }
}
