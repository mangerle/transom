//! 远程与本地文件分栏差异查看器：状态、异步读取、按变更块单向合并。
//!
//! 设计原理（Why）：
//! - 差异只读、写回显式：查看器本身不改任何文件；合并必须由用户点选某个变更块
//!   并二次确认，方向固定为"本地 → 远端"或"远端 → 本地"中的一条，不做双向自动合并；
//! - 远端读取复用编辑器的独立只读通道（`ReadText`），不占传输队列；
//!   远端写回复用 `WriteText` 的版本指纹与备份开关，和编辑器保存同一套安全契约；
//! - 本地读写是阻塞文件 IO，统一放到专用线程（`spawn_blocking_local`），
//!   结果回落到 UI 线程后再计算差异，避免大文件把 GPUI 主线程卡住；
//! - 差异计算产物是 hunk 区间，渲染交给组件库的 `Diff`（Split 模式）完成，
//!   控件只负责"上一处/下一处"与合并方向，不重复实现虚拟化与语法高亮。
//!
//! 代价与局限：
//! - 单次只能打开一个对比面板；再次打开会替换旧面板（未保存的仅是查看状态，
//!   不存在内容丢失）；
//! - 本地写回采用同目录临时文件 + 重命名保证原子性，不额外生成备份文件，
//!   需要历史版本时依赖用户自己的版本控制；
//! - 远端文件为非 UTF-8 编码（`transcoded`）时允许查看，但禁止写回。

mod compute;
mod local_io;
mod merge;
mod view;

#[cfg(test)]
mod tests;

use crate::app::AppShell;
use crate::format::format_size;
use gpui_kit::component::diff::DiffState;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use local_io::read_local_text;
use std::path::PathBuf;
use transom_core::{
    ConnectionId, FileDiff, MergeDirection, RemoteTextMeta, SftpCommand, SftpOperation, TabId,
    TransferId,
};

/// 单侧文件参与对比的字节上限；超过时拒绝而不是把两端内容都拉进内存。
pub(crate) const MAX_DIFF_BYTES: u64 = 4 * 1024 * 1024;

/// 查看器的阶段。读写期间禁止再次合并，避免同一份基线被并发覆盖。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffPhase {
    /// 至少一侧内容尚未读完。
    Loading,
    /// 两侧内容就绪，可跳转与合并。
    Ready,
    /// 正在写回某一侧。
    Writing,
}

/// 等待确认的一次合并。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiffConfirm {
    pub(crate) direction: MergeDirection,
    pub(crate) hunk: usize,
}

/// 本地对比入口的校验结果。
enum LocalDiffCheck {
    Ready(LocalDiffTarget),
    /// 本地终端没有远端文件：静默忽略。
    NotApplicable,
    /// 有条件不满足：把原因写进状态栏。
    Rejected(String),
}

struct LocalDiffTarget {
    connection_id: ConnectionId,
    local_path: PathBuf,
    remote_path: String,
}

/// 编辑器面板中参与对比的远端快照。
struct EditorDiffSnapshot {
    tab_id: TabId,
    remote_path: String,
    file_name: String,
    remote_text: String,
    remote_meta: Option<RemoteTextMeta>,
}

/// 一个对比面板的完整运行态。
pub(crate) struct DiffViewerState {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) local_path: PathBuf,
    pub(crate) remote_path: String,
    pub(crate) local_text: Option<String>,
    pub(crate) remote_text: Option<String>,
    pub(crate) remote_meta: Option<RemoteTextMeta>,
    pub(crate) diff: Option<FileDiff>,
    pub(crate) view: Option<Entity<DiffState>>,
    /// `DiffState` 中使用的文件路径，供按行滚动定位。
    pub(crate) diff_path: Option<SharedString>,
    pub(crate) hunk_index: Option<usize>,
    pub(crate) phase: DiffPhase,
    pub(crate) error: Option<String>,
    pub(crate) pending_read: Option<TransferId>,
    pub(crate) pending_write: Option<TransferId>,
    /// 在途远端写入的目标全文：成功事件只带回指纹，内容在这里暂存。
    pub(crate) pending_merged: Option<String>,
    /// 在途本地写入的目标全文。
    pub(crate) pending_local_merged: Option<String>,
    pub(crate) confirm: Option<DiffConfirm>,
    /// 最近一次差异计算的令牌：迟到的旧结果不得覆盖新结果。
    pub(crate) compute_token: TransferId,
}

impl DiffViewerState {
    fn new(
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        local_path: PathBuf,
        remote_path: String,
        pending_read: Option<TransferId>,
    ) -> Self {
        Self {
            tab_id,
            connection_id,
            local_path,
            remote_path,
            local_text: None,
            remote_text: None,
            remote_meta: None,
            diff: None,
            view: None,
            diff_path: None,
            hunk_index: None,
            phase: DiffPhase::Loading,
            error: None,
            pending_read,
            pending_write: None,
            pending_merged: None,
            pending_local_merged: None,
            confirm: None,
            compute_token: TransferId::new(),
        }
    }

    /// 远端是否允许写回；非 UTF-8 内容的编码边界不在这里猜。
    fn remote_write_blocker(&self) -> Result<(), &'static str> {
        match self.remote_meta {
            Some(meta) if meta.transcoded => Err("远端文件不是 UTF-8 文本，禁止写回"),
            Some(_) => Ok(()),
            None => Err("远端版本指纹缺失，请重新读取后再合并"),
        }
    }
}

impl AppShell {
    /// 本地栏对比入口是否可用：恰好选中一个本地文件，且远端当前目录有同名普通文件。
    pub(crate) fn diff_entry_ready(&self, tab_id: TabId) -> bool {
        matches!(self.local_diff_check(tab_id), LocalDiffCheck::Ready(_))
    }

    /// 校验"本地文件 → 远端同名文件"的对比前提；入口置灰与真正打开共用同一套口径。
    fn local_diff_check(&self, tab_id: TabId) -> LocalDiffCheck {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return LocalDiffCheck::NotApplicable;
        };
        let Some(connection_id) = tab.connection_id else {
            return LocalDiffCheck::NotApplicable;
        };
        let Some(panel) = tab.sftp() else {
            return LocalDiffCheck::NotApplicable;
        };
        let Some(local_path) = panel
            .local
            .selected
            .clone()
            .or_else(|| panel.local.selection.iter().next().cloned())
        else {
            return LocalDiffCheck::Rejected("请先选中一个本地文件".to_string());
        };
        if panel
            .local
            .entries
            .iter()
            .any(|entry| entry.path == local_path && entry.is_dir)
        {
            return LocalDiffCheck::Rejected("目录不能参与文件内容对比".to_string());
        }
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return LocalDiffCheck::Rejected("无法获取本地文件名".to_string());
        };
        let remote_path = join_remote(&panel.remote.path, &file_name);
        let Some(entry) = panel
            .remote
            .entries
            .iter()
            .find(|entry| entry.path == remote_path && !entry.is_dir)
        else {
            return LocalDiffCheck::Rejected(format!(
                "远端当前目录没有同名文件「{file_name}」，请先上传或选择对应文件"
            ));
        };
        if entry.size > MAX_DIFF_BYTES {
            return LocalDiffCheck::Rejected(format!(
                "远端文件超过对比上限 {}，请改用编辑器或先下载",
                format_size(MAX_DIFF_BYTES)
            ));
        }
        LocalDiffCheck::Ready(LocalDiffTarget {
            connection_id,
            local_path,
            remote_path,
        })
    }

    /// 从 SFTP 本地栏打开：选中本地文件，与远端当前目录的同名文件比对。
    pub(crate) fn sftp_open_local_diff(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let LocalDiffTarget {
            connection_id,
            local_path,
            remote_path,
        } = match self.local_diff_check(tab_id) {
            LocalDiffCheck::Ready(target) => target,
            LocalDiffCheck::NotApplicable => return,
            LocalDiffCheck::Rejected(reason) => {
                self.status = Some(reason);
                cx.notify();
                return;
            }
        };
        let max_bytes = MAX_DIFF_BYTES;
        let op_id = TransferId::new();
        self.diff_viewer = Some(DiffViewerState::new(
            tab_id,
            Some(connection_id),
            local_path,
            remote_path.clone(),
            Some(op_id),
        ));
        self.spawn_diff_local_read(cx);
        if !self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes,
            },
        }) && let Some(viewer) = self.diff_viewer.as_mut()
        {
            viewer.pending_read = None;
            viewer.phase = DiffPhase::Ready;
            viewer.error = Some("读取远端文件的命令投递失败".to_string());
        }
        cx.notify();
    }

    /// 从编辑器打开：用编辑器当前基线（远端实际内容）与选中的本地文件比对。
    pub(crate) fn open_editor_local_diff(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.editor_diff_snapshot(editor) else {
            return;
        };
        let EditorDiffSnapshot {
            tab_id,
            remote_path,
            file_name,
            remote_text,
            remote_meta,
        } = snapshot;
        let Some(connection_id) = self.workspace.tab(tab_id).and_then(|tab| tab.connection_id)
        else {
            self.status = Some("所属连接已断开，无法对比".to_string());
            cx.notify();
            return;
        };
        let Some(local_path) = self.pick_local_diff_file(&file_name, cx) else {
            return;
        };
        let mut viewer =
            DiffViewerState::new(tab_id, Some(connection_id), local_path, remote_path, None);
        viewer.remote_text = Some(remote_text);
        viewer.remote_meta = remote_meta;
        self.diff_viewer = Some(viewer);
        self.spawn_diff_local_read(cx);
        self.recompute_diff(cx);
        cx.notify();
    }

    /// 取编辑器面板中可参与对比的远端快照；未装载、新建文件或读写中返回 `None`。
    fn editor_diff_snapshot(&self, editor: &Entity<EditorState>) -> Option<EditorDiffSnapshot> {
        self.editors
            .panels
            .iter()
            .find(|item| item.editor == *editor)
            .filter(|item| {
                item.loaded
                    && !item.new_file
                    && !matches!(
                        item.phase,
                        crate::editor::EditorPhase::Reading | crate::editor::EditorPhase::Writing
                    )
            })
            .map(|item| EditorDiffSnapshot {
                tab_id: item.tab_id,
                remote_path: item.remote_path.clone(),
                file_name: item.file_name.clone(),
                remote_text: item.baseline.to_string(),
                remote_meta: item.remote_meta,
            })
    }

    /// 选择参与对比的本地文件；取消返回 `None`，条件不满足时写状态栏并返回 `None`。
    fn pick_local_diff_file(
        &mut self,
        remote_name: &str,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        let local_path = rfd::FileDialog::new()
            .set_title(format!("选择与「{remote_name}」对比的本地文件"))
            .pick_file()?;
        match std::fs::metadata(&local_path) {
            Ok(meta) if meta.len() > MAX_DIFF_BYTES => {
                self.status = Some(format!(
                    "本地文件超过对比上限 {}",
                    format_size(MAX_DIFF_BYTES)
                ));
            }
            Ok(meta) if !meta.is_file() => {
                self.status = Some("请选择普通文件".to_string());
            }
            Err(error) => {
                self.status = Some(format!("无法读取本地文件属性: {error}"));
            }
            Ok(_) => return Some(local_path),
        }
        cx.notify();
        None
    }

    fn spawn_diff_local_read(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.diff_viewer.as_ref() else {
            return;
        };
        let path = viewer.local_path.clone();
        cx.spawn(async move |this, cx| {
            let result = crate::sftp::spawn_blocking_local(move || read_local_text(&path)).await;
            let _ = this.update(cx, |this, cx| {
                this.apply_diff_local_read(result, cx);
            });
        })
        .detach();
    }

    fn apply_diff_local_read(
        &mut self,
        result: Option<Result<String, String>>,
        cx: &mut Context<Self>,
    ) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        match result {
            Some(Ok(text)) => {
                viewer.local_text = Some(text);
                viewer.error = None;
            }
            Some(Err(error)) => {
                viewer.error = Some(error);
                viewer.phase = DiffPhase::Ready;
            }
            None => {
                viewer.error = Some("本地文件读取线程启动失败".to_string());
                viewer.phase = DiffPhase::Ready;
            }
        }
        self.recompute_diff(cx);
        cx.notify();
    }

    pub(crate) fn close_diff_viewer(&mut self, cx: &mut Context<Self>) {
        self.diff_viewer = None;
        cx.notify();
    }
}

/// 远端目录与文件名的拼接：根目录不能出现双斜杠。
pub(super) fn join_remote(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}

/// diff 头部标签里不能出现换行；路径来自用户文件系统，防御性替换。
pub(super) fn sanitize_label(label: &str) -> String {
    label.replace(['\n', '\r', '\t'], " ")
}
