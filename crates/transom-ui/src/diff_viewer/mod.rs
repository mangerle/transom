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

mod actions;
mod compute;
mod entity;
mod local_io;
mod merge;
mod view;

#[cfg(test)]
mod tests;

use crate::app::AppShell;
use crate::editor::EditorDiffSource;
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
    fn remote_write_blocker(&self) -> Result<(), String> {
        match self.remote_meta {
            Some(meta) if meta.transcoded => {
                Err(rust_i18n::t!("diff.error_remote_not_utf8").to_string())
            }
            Some(_) => Ok(()),
            None => Err(rust_i18n::t!("diff.error_remote_meta_missing").to_string()),
        }
    }
}

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
        return LocalDiffCheck::Rejected(rust_i18n::t!("diff.error_no_local_file").to_string());
    };
    if panel
        .local
        .entries
        .iter()
        .any(|entry| entry.path == local_path && entry.is_dir)
    {
        return LocalDiffCheck::Rejected(rust_i18n::t!("diff.error_local_is_dir").to_string());
    }
    let Some(file_name) = local_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
    else {
        return LocalDiffCheck::Rejected(rust_i18n::t!("diff.error_local_no_name").to_string());
    };
    let remote_path = join_remote(&panel.remote.path, &file_name);
    let Some(entry) = panel
        .remote
        .entries
        .iter()
        .find(|entry| entry.path == remote_path && !entry.is_dir)
    else {
        return LocalDiffCheck::Rejected(
            rust_i18n::t!("diff.error_remote_missing", name = file_name).to_string(),
        );
    };
    if entry.size > MAX_DIFF_BYTES {
        return LocalDiffCheck::Rejected(
            rust_i18n::t!("diff.error_remote_too_large", limit = MAX_DIFF_BYTES).to_string(),
        );
    }
    LocalDiffCheck::Ready(LocalDiffTarget {
        connection_id,
        local_path,
        remote_path,
    })
}

impl DiffViewerView {
    /// 启动本地文件读取；结果回到实体后触发重算。
    fn spawn_local_read(&mut self, cx: &mut Context<Self>) {
        let path = self.state.local_path.clone();
        cx.spawn(async move |this, cx| {
            let result = crate::app::spawn_blocking_local(move || read_local_text(&path)).await;
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
                self.state.error = Some(rust_i18n::t!("diff.error_local_read_spawn").to_string());
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
