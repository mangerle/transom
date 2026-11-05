//! SFTP 双栏文件面板状态、选择状态机与各模态弹窗状态数据结构。

use super::columns::{ColumnWidthMemory, SftpColumnId};
use super::overlay::DropOverlayState;
use super::preview::SftpPreviewState;
use super::quick_paths::LocalQuickPaths;
use super::side_remote::{SftpActivePane, SftpClipboardData};
use super::task::TransferTask;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
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
    /// 目标连接；`None` 表示标签页主远端，`Some` 表示并列远端列。
    pub(crate) connection_id: Option<ConnectionId>,
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
#[derive(Clone)]
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
    /// 远端归档压缩：在当前目录下生成归档文件
    RemoteArchive {
        source_paths: Vec<String>,
        parent_dir: String,
    },
    /// 远端解压到指定新目录
    RemoteExtract {
        archive_path: String,
        parent_dir: String,
    },
    /// Sudo 提权保存：输入提权密码并写入远端
    SudoElevation {
        editor: Entity<gpui_kit::component::input::EditorState>,
    },
}

pub(crate) struct SftpInputModalState {
    pub(crate) tab_id: TabId,
    /// 远端目标连接；`None` 表示标签页主远端，`Some` 表示并列远端列。
    /// 本地目标不使用该字段。
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) target: SftpInputTarget,
    pub(crate) error: Option<String>,
}

/// 远程目录条目短期缓存存活时间：60 秒。
pub(crate) const SFTP_DIR_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// 远程目录条目短期缓存项：条目快照与缓存时间戳。
#[derive(Clone, Debug)]
pub(crate) struct RemoteDirCacheEntry {
    pub(crate) entries: Vec<RemoteFile>,
    pub(crate) cached_at: std::time::Instant,
}

pub(crate) use super::remote_pane_state::{SftpRemotePaneRef, SftpRemotePaneState};

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
    /// 本列是否显示隐藏文件。
    pub(crate) show_hidden: bool,
    /// 本列是否展开过滤输入框。
    pub(crate) show_filter: bool,
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

/// 本地栏的构造参数：面板初始化与本地单栏共用同一套字段默认值。
pub(crate) struct SftpLocalPaneInit {
    pub(crate) path: PathBuf,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) quick_paths: LocalQuickPaths,
    pub(crate) only: bool,
    pub(crate) error: Option<String>,
}

impl SftpLocalPaneState {
    pub(crate) fn new(init: SftpLocalPaneInit) -> Self {
        Self {
            path: init.path,
            path_input: init.path_input,
            filter_input: init.filter_input,
            entries: Vec::new(),
            quick_paths: init.quick_paths,
            only: init.only,
            show_hidden: false,
            show_filter: false,
            collapsed: false,
            selected: None,
            selection: BTreeSet::new(),
            anchor: None,
            error: init.error,
            scroll_handle: VirtualListScrollHandle::new(),
        }
    }
}

/// 传输队列与运行态：任务集合、并发控制、进度与瞬时消息。
pub(crate) struct SftpQueueState {
    pub(crate) tasks: Vec<TransferTask>,
    /// 当前正在由队列执行的任务集合，支持并行传输。
    pub(crate) active_transfers: HashSet<TransferId>,
    pub(crate) progress: Option<(u64, Option<u64>)>,
    /// 断点续传开关：只影响“被中断任务重新排队时是否沿用已传进度”，
    /// 全新任务与目标端已有同名文件无关，永远整份覆盖。
    pub(crate) resume_enabled: bool,
    pub(crate) expanded: bool,
    pub(crate) height: f32,
    pub(crate) drag_start_y: Option<f32>,
    pub(crate) drag_start_height: f32,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    /// 瞬时状态消息的过期任务；替换消息时旧任务被取消。
    pub(super) status_task: Option<Task<()>>,
    /// 错误提示的过期任务；清除或替换错误时旧任务被取消。
    pub(super) error_task: Option<Task<()>>,
    /// 终端右下角上传浮层的状态（面板未展开时用它承接进度可见性）。
    pub(crate) drop_overlay: DropOverlayState,
}

/// SFTP 多列面板状态：主远端、本地栏、传输队列与并列远端列。
///
/// 设计原理（Why）：此前 47 个字段平铺在同一个结构体里，路径、选择、队列
/// 与展开状态互相穿插，任何模块都能直接改写任意字段；按"远端列 / 本地栏 /
/// 传输队列"分组后，字段归属一眼可见，读写落在各自的子结构上。
/// 可挂载到并列列的远端候选。
#[derive(Clone)]
pub(crate) struct SftpMountCandidate {
    pub(crate) connection_id: ConnectionId,
    pub(crate) display_name: String,
    pub(crate) view: Option<Entity<crate::sftp::SftpPanelView>>,
    pub(crate) cwd: Option<String>,
}

pub(crate) struct SftpPanelState {
    /// 面板所属主连接：打开面板时由外壳注入，重连后更新。
    ///
    /// 面板靠它解析"主远端列该用哪条连接"，从而不必在动作与渲染里反查外壳。
    pub(crate) main_connection: Option<ConnectionId>,
    /// 命令通道：面板自己就能投递命令，不必回读外壳服务。
    pub(crate) command_tx: Option<flume::Sender<transom_core::AppCommand>>,
    /// 编辑面板实体句柄：远端工具栏要渲染"已打开编辑器"浮层，需要读取它。
    pub(crate) editors: Option<Entity<crate::editor::EditorPanels>>,
    /// 可挂载的并列远端候选：外壳在标签变化时推送，面板渲染不再读工作区。
    pub(crate) mount_candidates: Vec<SftpMountCandidate>,
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
    pub(crate) side_remotes: Vec<SftpRemotePaneState>,
    /// 跨列文件剪贴板（Ctrl+C / Ctrl+V）。
    pub(crate) clipboard: Option<SftpClipboardData>,
    /// 挂载新远端浮层菜单是否打开。
    pub(crate) mount_menu_open: bool,
    /// 主远端导航是否联动本地列。
    ///
    /// 只属于"主远端 ↔ 本地"这条关系；并列列不显示该开关，避免一列切换
    /// 让所有列的按钮同时变亮。
    pub(crate) sync_navigation: bool,
    pub(crate) active_pane: SftpActivePane,
    pub(crate) focus_handle: FocusHandle,
    /// 当前悬停预览；同一时刻只保留一个目标，切换目标或离开行时整体释放。
    pub(crate) hover_preview: Option<SftpPreviewState>,
    /// 多列分栏状态：由面板自己持有，跨帧保留用户拖出来的列宽。
    pub(crate) column_state: Entity<ResizableState>,
    /// 各列宽度记忆（像素，按列身份），折叠时展开列按它固定宽度。
    pub(crate) column_widths: ColumnWidthMemory,
    /// 待由渲染期消费的一次性宽度调整意图（折叠时收缩、展开时向外加宽）。
    pub(crate) pending_width_adjustment: Option<SftpWidthAdjustment>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

/// SFTP 面板宽度调整意图：仅在折叠/展开动作发生时由渲染期一次性消费。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SftpWidthAdjustment {
    /// 在当前实际面板宽度基础上调整 delta（正数加宽，负数收窄）。
    Delta(f32),
    /// 强制设定为特定宽度（例如所有列全收起时设定为折叠条总宽）。
    Exact(f32),
}

impl SftpPanelState {
    /// 当前展开列的身份，顺序与渲染一致（本地 | 远端 | 并列远端）。
    ///
    /// 宽度记忆按下标对应分栏组的面板顺序，而分栏组里只有展开的列，
    /// 所以这里给出的顺序必须与渲染时构造分栏组的顺序严格一致。
    pub(crate) fn expanded_column_ids(&self) -> Vec<SftpColumnId> {
        let mut ids = Vec::new();
        if !self.local.collapsed {
            ids.push(SftpColumnId::Local);
        }
        if !self.local.only && !self.remote.collapsed {
            ids.push(SftpColumnId::Remote);
        }
        for (index, side) in self.side_remotes.iter().enumerate() {
            if !side.collapsed {
                ids.push(SftpColumnId::Side(index));
            }
        }
        ids
    }

    /// 设置面板级状态消息；后续消息会整体替换，不做自动过期。
    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.queue.status = Some(message.into());
    }

    /// 清除面板级状态消息。
    pub(crate) fn clear_status(&mut self) {
        self.queue.status = None;
    }

    /// 设置传输队列错误消息；由队列错误栏展示，成功路径或过期任务负责清除。
    ///
    /// 远端列自身的操作错误落在对应列的 `error` 上，这里只承载队列/传输语义。
    pub(crate) fn set_queue_error(&mut self, message: impl Into<String>) {
        self.queue.error = Some(message.into());
    }

    /// 清除传输队列错误消息，并取消它的过期任务。
    pub(crate) fn clear_queue_error(&mut self) {
        self.queue.error = None;
        self.queue.error_task = None;
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

mod select;
