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
mod entity;
mod local_io;
mod merge;
mod view;

#[cfg(test)]
mod tests;

use crate::app::{AppShell, AppState};
use crate::editor::EditorDiffSource;
use crate::format::format_size;
pub(crate) use entity::{DiffViewerEvent, DiffViewerView};
use gpui_kit::component::diff::DiffState;
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
pub(crate) enum LocalDiffCheck {
    Ready(LocalDiffTarget),
    /// 本地终端没有远端文件：静默忽略。
    NotApplicable,
    /// 有条件不满足：把原因写进状态栏。
    Rejected(String),
}

#[derive(Clone)]
pub(crate) struct LocalDiffTarget {
    connection_id: ConnectionId,
    local_path: PathBuf,
    remote_path: String,
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

impl AppState {}

/// 校验"本地文件 → 远端同名文件"的对比前提；入口置灰与真正打开共用同一套口径。
///
/// 只依赖连接号与面板状态，面板实体可直接调用，渲染期不必反查外壳。
pub(crate) fn local_diff_check(
    connection_id: Option<transom_core::ConnectionId>,
    panel: &crate::sftp::SftpPanelState,
) -> LocalDiffCheck {
    let Some(connection_id) = connection_id else {
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

impl AppShell {
    /// 已通过校验的本地对比目标：创建对比面板并请求远端文本。
    ///
    /// 不读取实体：SFTP 实体在回调里完成校验后只把目标交回来。
    pub(crate) fn open_local_diff_with_target(
        &mut self,
        tab_id: TabId,
        target: LocalDiffTarget,
        cx: &mut Context<Self>,
    ) {
        let LocalDiffTarget {
            connection_id,
            local_path,
            remote_path,
        } = target;
        let max_bytes = MAX_DIFF_BYTES;
        let op_id = TransferId::new();
        let view = self.install_diff_viewer(
            DiffViewerState::new(
                tab_id,
                Some(connection_id),
                local_path,
                remote_path.clone(),
                Some(op_id),
            ),
            cx,
        );
        view.update(cx, |view, cx| view.spawn_local_read(cx));
        let sent = self.state.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes,
            },
        });
        if !sent {
            view.update(cx, |view, cx| {
                view.state.pending_read = None;
                view.state.phase = DiffPhase::Ready;
                view.state.error = Some("读取远端文件的命令投递失败".to_string());
                view.notify_changed(cx);
            });
        }
        cx.notify();
    }

    /// 从编辑器打开：用编辑器当前基线（远端实际内容）与选中的本地文件比对。
    /// 打开"与本地对比"面板。
    ///
    /// 远端快照由编辑面板实体在回调里先算好再传入：本方法运行在实体回调的
    /// 回交路径上，禁止再 `read`/`update` 该实体，否则会双重租借崩溃。
    pub(crate) fn open_editor_local_diff(
        &mut self,
        source: EditorDiffSource,
        cx: &mut Context<Self>,
    ) {
        let EditorDiffSource {
            tab_id,
            remote_path,
            file_name,
            remote_text,
            remote_meta,
        } = source;
        let Some(connection_id) = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id)
        else {
            self.state.status = Some("所属连接已断开，无法对比".to_string());
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
        let view = self.install_diff_viewer(viewer, cx);
        view.update(cx, |view, cx| view.spawn_local_read(cx));
        cx.notify();
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
                self.state.status = Some(format!(
                    "本地文件超过对比上限 {}",
                    format_size(MAX_DIFF_BYTES)
                ));
            }
            Ok(meta) if !meta.is_file() => {
                self.state.status = Some("请选择普通文件".to_string());
            }
            Err(error) => {
                self.state.status = Some(format!("无法读取本地文件属性: {error}"));
            }
            Ok(_) => return Some(local_path),
        }
        cx.notify();
        None
    }

    pub(crate) fn close_diff_viewer(&mut self, cx: &mut Context<Self>) {
        self.state.diff_viewer = None;
        self.state.tasks.subscriptions.diff_viewer = None;
        cx.notify();
    }

    /// 创建对比实体并订阅其状态事件；同一时刻只保留一个对比面板。
    fn install_diff_viewer(
        &mut self,
        viewer: DiffViewerState,
        cx: &mut Context<Self>,
    ) -> Entity<DiffViewerView> {
        let view = cx.new(|_| {
            DiffViewerView::new(
                viewer,
                self.state.services.command_tx.clone(),
                self.state.editors.clone(),
            )
        });
        self.state.diff_viewer = Some(view.clone());
        let subscription =
            cx.subscribe(
                &view,
                |shell, _view, event: &DiffViewerEvent, cx| match event {
                    DiffViewerEvent::Changed => cx.notify(),
                    DiffViewerEvent::RemoteEntryUpdated {
                        tab_id,
                        path,
                        size,
                        modified,
                    } => shell.update_diff_remote_entry(*tab_id, path, *size, *modified, cx),
                },
            );
        self.state.tasks.subscriptions.diff_viewer = Some(subscription);
        view
    }

    fn update_diff_remote_entry(
        &mut self,
        tab_id: TabId,
        path: &str,
        size: u64,
        modified: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if let Some(mut panel) = self.sftp_view_mut(tab_id, cx)
            && let Some(entry) = panel
                .state
                .remote
                .entries
                .iter_mut()
                .find(|entry| entry.path == path)
        {
            entry.size = size;
            entry.modified = modified;
        }
        cx.notify();
    }

    /// 转发远端读取完成事件到对比实体。
    pub(crate) fn diff_handle_remote_read(
        &mut self,
        op_id: TransferId,
        content: &str,
        meta: RemoteTextMeta,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.state.diff_viewer.clone() else {
            return false;
        };
        let handled = view.update(cx, |view, cx| {
            view.diff_handle_remote_read(op_id, content, meta, cx)
        });
        if handled {
            cx.notify();
        }
        handled
    }

    /// 转发远端写回完成事件到对比实体。
    pub(crate) fn diff_handle_remote_written(
        &mut self,
        op_id: TransferId,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.state.diff_viewer.clone() else {
            return false;
        };
        let handled = view.update(cx, |view, cx| {
            view.diff_handle_remote_written(op_id, meta, window, cx)
        });
        if handled {
            cx.notify();
        }
        handled
    }

    /// 转发远端失败事件到对比实体。
    pub(crate) fn diff_handle_remote_failure(
        &mut self,
        op_id: TransferId,
        message: &str,
        conflict: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.state.diff_viewer.clone() else {
            return false;
        };
        let handled = view.update(cx, |view, cx| {
            view.diff_handle_remote_failure(op_id, message, conflict, cx)
        });
        if handled {
            cx.notify();
        }
        handled
    }

    /// 适配对比工具栏：上一处/下一处。
    pub(crate) fn step_diff_hunk(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(view) = self.state.diff_viewer.clone() else {
            return;
        };
        view.update(cx, |view, cx| view.step_diff_hunk(forward, cx));
        cx.notify();
    }

    /// 适配对比工具栏：请求合并。
    pub(crate) fn request_diff_merge(&mut self, direction: MergeDirection, cx: &mut Context<Self>) {
        let Some(view) = self.state.diff_viewer.clone() else {
            return;
        };
        view.update(cx, |view, cx| view.request_diff_merge(direction, cx));
        cx.notify();
    }

    /// 适配对比工具栏：取消合并确认。
    pub(crate) fn cancel_diff_merge(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.state.diff_viewer.clone() else {
            return;
        };
        view.update(cx, |view, cx| view.cancel_diff_merge(cx));
        cx.notify();
    }

    /// 适配对比工具栏：确认合并。
    pub(crate) fn confirm_diff_merge(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.state.diff_viewer.clone() else {
            return;
        };
        view.update(cx, |view, cx| view.confirm_diff_merge(cx));
        cx.notify();
    }
}

impl DiffViewerView {
    /// 启动本地文件读取；结果回到实体后触发重算。
    fn spawn_local_read(&mut self, cx: &mut Context<Self>) {
        let path = self.state.local_path.clone();
        cx.spawn(async move |this, cx| {
            let result = crate::sftp::spawn_blocking_local(move || read_local_text(&path)).await;
            let _ = this.update(cx, |this, cx| this.apply_local_read(result, cx));
        })
        .detach();
    }

    fn apply_local_read(&mut self, result: Option<Result<String, String>>, cx: &mut Context<Self>) {
        match result {
            Some(Ok(text)) => {
                self.state.local_text = Some(text);
                self.state.error = None;
            }
            Some(Err(error)) => {
                self.state.error = Some(error);
                self.state.phase = DiffPhase::Ready;
            }
            None => {
                self.state.error = Some("本地文件读取线程启动失败".to_string());
                self.state.phase = DiffPhase::Ready;
            }
        }
        self.recompute_diff(cx);
        self.notify_changed(cx);
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
