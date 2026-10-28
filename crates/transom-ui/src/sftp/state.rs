//! SFTP 双栏文件面板状态、选择状态机与各模态弹窗状态数据结构。

use super::overlay::DropOverlayState;
use super::preview::SftpPreviewState;
use super::quick_paths::LocalQuickPaths;
use super::selection;
use super::side_remote::{SftpActivePane, SftpClipboardData, SftpSideRemoteState};
use super::task::TransferTask;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use transom_core::{ConnectionId, DirDiffEntry, RemoteFile, TabId, TransferId};

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
    /// 新建本地文件；提交后在当前本地目录下创建空文件并选中。
    LocalNewFile,
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
/// 远端栏（主栏）的导航、条目与选择状态。
pub(crate) struct SftpRemotePaneState {
    pub(crate) path: String,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) name_input: Entity<InputState>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) entries: Vec<RemoteFile>,
    pub(crate) selected: Option<String>,
    /// 多选集合；`selected` 始终指向其中最近操作的主项。
    pub(crate) selection: BTreeSet<String>,
    /// Shift 连选的锚点路径。
    pub(crate) anchor: Option<String>,
    pub(crate) scroll_handle: VirtualListScrollHandle,
    /// 是否折叠为竖条。
    pub(crate) collapsed: bool,
    pub(crate) loading: bool,
}

/// 本地栏的导航、条目与选择状态。
pub(crate) struct SftpLocalPaneState {
    pub(crate) path: PathBuf,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) entries: Vec<LocalFileEntry>,
    /// 打开面板时解析一次的常用目录，避免逐帧读取 XDG 配置。
    pub(crate) quick_paths: LocalQuickPaths,
    /// 本地终端专用：只渲染本地栏，不包含远端、队列与上传入口。
    pub(crate) only: bool,
    /// 是否折叠为竖条。
    pub(crate) collapsed: bool,
    pub(crate) selected: Option<PathBuf>,
    /// 多选集合；`selected` 始终指向其中最近操作的主项。
    pub(crate) selection: BTreeSet<PathBuf>,
    /// Shift 连选的锚点路径。
    pub(crate) anchor: Option<PathBuf>,
    pub(crate) error: Option<String>,
    pub(crate) scroll_handle: VirtualListScrollHandle,
}

/// 传输队列与运行态：任务集合、并发控制、进度与瞬时消息。
pub(crate) struct SftpQueueState {
    pub(crate) tasks: Vec<TransferTask>,
    /// 当前正在由队列执行的任务集合，支持并行传输。
    pub(crate) active_transfers: HashSet<TransferId>,
    pub(crate) active_op: Option<TransferId>,
    pub(crate) progress: Option<(u64, Option<u64>)>,
    pub(crate) resume_enabled: bool,
    pub(crate) expanded: bool,
    pub(crate) height: f32,
    pub(crate) drag_start_y: Option<f32>,
    pub(crate) drag_start_height: f32,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    /// 瞬时状态消息的过期任务；替换消息时旧任务被取消。
    pub(super) status_task: Option<Task<()>>,
    /// 终端右下角上传浮层的状态（面板未展开时用它承接进度可见性）。
    pub(crate) drop_overlay: DropOverlayState,
}

/// SFTP 双栏面板状态：远端栏、本地栏与传输队列三块内聚子状态，
/// 外加并排远端、剪贴板与面板级交互标记。
///
/// 设计原理（Why）：此前 47 个字段平铺在同一个结构体里，路径、选择、队列
/// 与展开状态互相穿插，任何模块都能直接改写任意字段；按"远端栏 / 本地栏 /
/// 传输队列"分组后，字段归属一眼可见，读写落在各自的子结构上。
pub(crate) struct SftpPanelState {
    /// 面板列是否展开显示。
    ///
    /// 设计原理（Why）：`tab.sftp` 是 `Some` 只代表传输队列与目录状态存在，
    /// 不再等于"面板开着"——拖文件到终端上传要在不打开面板的前提下也持有队列，
    /// 因此建出状态、把进度改由终端右下角浮层呈现。凡是决定"要不要渲染面板列/
    /// 活动栏是否高亮"的地方一律读本字段，而不是读 `Option` 本身。
    pub(crate) open: bool,
    pub(crate) remote: SftpRemotePaneState,
    pub(crate) local: SftpLocalPaneState,
    pub(crate) queue: SftpQueueState,
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
    pub(crate) focus_handle: FocusHandle,
    /// 当前悬停预览；同一时刻只保留一个目标，切换目标或离开行时整体释放。
    pub(crate) hover_preview: Option<SftpPreviewState>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl SftpPanelState {
    /// 设置面板级状态消息；后续消息会整体替换，不做自动过期。
    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.queue.status = Some(message.into());
    }

    /// 清除面板级状态消息。
    pub(crate) fn clear_status(&mut self) {
        self.queue.status = None;
    }

    /// 设置面板级错误消息；由错误浮层展示，成功路径负责清除。
    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        self.queue.error = Some(message.into());
    }

    /// 清除面板级错误消息。
    pub(crate) fn clear_error(&mut self) {
        self.queue.error = None;
    }

    /// 设置本地栏错误消息（本地文件系统操作专用）。
    pub(crate) fn set_local_error(&mut self, message: impl Into<String>) {
        self.local.error = Some(message.into());
    }

    /// 清除本地栏错误消息。
    pub(crate) fn clear_local_error(&mut self) {
        self.local.error = None;
    }
}

impl SftpPanelState {
    /// 选中远程条目并标记远程栏为活动栏（行点击与右键菜单共用）。
    ///
    /// 设计原理（Why）：Delete 等快捷键按“活动栏”定位目标，若各处分散直接写
    /// `selected` 字段，右键操作另一栏后会残留旧活动栏，导致快捷键删错栏位；
    /// 统一经本方法赋值可从机制上杜绝该失配。
    pub(crate) fn select_remote(&mut self, path: String) {
        selection::select(
            &mut self.remote.selected,
            &mut self.remote.selection,
            &mut self.remote.anchor,
            path,
        );
        self.active_pane = SftpActivePane::Remote;
    }

    /// 设置远端主项但保留已有多选集合，供右键菜单定位单项操作。
    pub(crate) fn set_remote_primary(&mut self, path: String) {
        selection::set_primary(&mut self.remote.selected, &mut self.remote.selection, path);
        self.active_pane = SftpActivePane::Remote;
    }

    /// Ctrl/Cmd 点击切换远端条目的选中状态。
    pub(crate) fn toggle_remote_selection(&mut self, path: String) {
        selection::toggle(
            &mut self.remote.selected,
            &mut self.remote.selection,
            &mut self.remote.anchor,
            path,
        );
        self.active_pane = SftpActivePane::Remote;
    }

    /// Shift 连选：用可见列表中的一段路径替换远端集合。
    pub(crate) fn select_remote_range(
        &mut self,
        paths: impl IntoIterator<Item = String>,
        primary: String,
    ) {
        selection::select_range(
            &mut self.remote.selected,
            &mut self.remote.selection,
            paths,
            primary,
        );
        self.active_pane = SftpActivePane::Remote;
    }

    /// 返回当前需要批量操作的远端路径；无多选时退化为单选主项。
    pub(crate) fn selected_remote_paths(&self) -> Vec<String> {
        selection::paths(&self.remote.selected, &self.remote.selection)
    }

    pub(crate) fn clear_remote_selection(&mut self) {
        selection::clear(
            &mut self.remote.selected,
            &mut self.remote.selection,
            &mut self.remote.anchor,
        );
    }

    /// 刷新远端列表后剔除已不存在的选中项，并修正主项与锚点。
    pub(crate) fn sync_remote_selection(&mut self, valid: &BTreeSet<String>) {
        selection::sync(
            &mut self.remote.selected,
            &mut self.remote.selection,
            &mut self.remote.anchor,
            valid,
        );
    }

    /// 单选一个本地条目，同时清空多选集合并重置 Shift 锚点。
    pub(crate) fn select_local(&mut self, path: PathBuf) {
        selection::select(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
            path,
        );
        self.active_pane = SftpActivePane::Local;
    }

    /// 设置本地主项但保留已有多选集合，供右键菜单定位单项操作。
    pub(crate) fn set_local_primary(&mut self, path: PathBuf) {
        selection::set_primary(&mut self.local.selected, &mut self.local.selection, path);
        self.active_pane = SftpActivePane::Local;
    }

    /// Ctrl/Cmd 点击切换单个条目的选中状态。
    pub(crate) fn toggle_local_selection(&mut self, path: PathBuf) {
        selection::toggle(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
            path,
        );
        self.active_pane = SftpActivePane::Local;
    }

    /// Shift 连选：用可见列表中的一段路径替换当前集合。
    pub(crate) fn select_local_range(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        primary: PathBuf,
    ) {
        selection::select_range(
            &mut self.local.selected,
            &mut self.local.selection,
            paths,
            primary,
        );
        self.active_pane = SftpActivePane::Local;
    }

    /// 返回当前需要批量操作的本地路径；无多选时退化为单选主项。
    pub(crate) fn selected_local_paths(&self) -> Vec<PathBuf> {
        selection::paths(&self.local.selected, &self.local.selection)
    }

    pub(crate) fn clear_local_selection(&mut self) {
        selection::clear(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
        );
    }

    /// 刷新目录后剔除已不存在的选中项，并修正主项与锚点。
    pub(crate) fn sync_local_selection(&mut self, valid: &BTreeSet<PathBuf>) {
        selection::sync(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
            valid,
        );
    }

    /// 汇总所有活动任务的字节进度；无活动任务时清空进度。
    pub(crate) fn recompute_progress(&mut self) {
        if self.queue.active_transfers.is_empty() {
            self.queue.progress = None;
            return;
        }
        let mut transferred = 0u64;
        let mut total = 0u64;
        let mut total_known = true;
        for task in &self.queue.tasks {
            if !self.queue.active_transfers.contains(&task.id) {
                continue;
            }
            transferred = transferred.saturating_add(task.transferred);
            match task.size {
                Some(size) => total = total.saturating_add(size),
                None => total_known = false,
            }
        }
        self.queue.progress = Some((transferred, total_known.then_some(total)));
    }

    /// 是否所有文件列均已处于折叠收起状态。
    pub(crate) fn is_all_collapsed(&self) -> bool {
        if !self.local.collapsed {
            return false;
        }
        if !self.local.only && !self.remote.collapsed {
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
        if self.local.collapsed {
            count += 1;
        }
        if !self.local.only && self.remote.collapsed {
            count += 1;
        }
        count += self.side_remotes.iter().filter(|s| s.collapsed).count();
        count
    }
}
