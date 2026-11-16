//! 对比面板的外壳协调：安装实体、转发远端事件与工具栏适配。
//!
//! 设计原理（Why）：对比状态与渲染归 [`super::DiffViewerView`]；本模块只借用
//! 对比实体句柄、订阅槽、命令通道与工作区快照，不再接收整个 `AppShell`。

use super::*;
use crate::app::{AppState, PendingNotices};
use crate::editor::EditorPanels;
use crate::workspace::WorkspaceState;
use gpui_kit::{Context, Entity, Subscription, Window};
use std::path::PathBuf;
use transom_core::{AppCommand, TransferId};

/// 对比面板协调所需的窄上下文。
pub(crate) struct DiffViewerOps<'a> {
    diff_viewer: &'a mut Option<Entity<DiffViewerView>>,
    diff_viewer_subscription: &'a mut Option<Subscription>,
    command_tx: &'a flume::Sender<AppCommand>,
    editors: &'a Entity<EditorPanels>,
    workspace: &'a WorkspaceState,
    pending_notices: &'a mut PendingNotices,
    status: &'a mut Option<String>,
}

impl<'a> DiffViewerOps<'a> {
    /// 从应用状态拆借对比域能力。
    pub(crate) fn new(app: &'a mut AppState) -> Self {
        Self {
            diff_viewer: &mut app.diff_viewer,
            diff_viewer_subscription: &mut app.tasks.subscriptions.diff_viewer,
            command_tx: &app.services.command_tx,
            editors: &app.editors,
            workspace: &app.workspace,
            pending_notices: &mut app.feedback.pending_notices,
            status: &mut app.status,
        }
    }

    /// 打开本地文件与远端路径的对比面板并请求远端文本。
    pub(crate) fn open_local_diff_with_target(
        &mut self,
        tab_id: TabId,
        target: LocalDiffTarget,
        cx: &mut Context<AppShell>,
    ) {
        let LocalDiffTarget {
            connection_id,
            local_path,
            remote_path,
        } = target;
        let op_id = TransferId::new();
        let view = self.install(
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
        let sent = self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes: MAX_DIFF_BYTES,
            },
        });
        if !sent {
            view.update(cx, |view, cx| {
                view.state.pending_read = None;
                view.state.phase = DiffPhase::Ready;
                view.state.error = Some(rust_i18n::t!("diff.error_read_dispatch").to_string());
                view.notify_changed(cx);
            });
        }
        cx.notify();
    }

    /// 从编辑器当前基线与选中的本地文件打开对比面板。
    pub(crate) fn open_editor_local_diff(
        &mut self,
        source: EditorDiffSource,
        cx: &mut Context<AppShell>,
    ) {
        let EditorDiffSource {
            tab_id,
            remote_path,
            file_name,
            remote_text,
            remote_meta,
        } = source;
        let Some(connection_id) = self.workspace.tab(tab_id).and_then(|tab| tab.connection_id)
        else {
            *self.status = Some(rust_i18n::t!("diff.error_no_connection").to_string());
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
        let view = self.install(viewer, cx);
        view.update(cx, |view, cx| view.spawn_local_read(cx));
        cx.notify();
    }

    /// 关闭对比面板并回收事件订阅。
    pub(crate) fn close(&mut self, cx: &mut Context<AppShell>) {
        *self.diff_viewer = None;
        *self.diff_viewer_subscription = None;
        cx.notify();
    }

    /// 转发远端读取完成事件到对比实体。
    pub(crate) fn handle_remote_read(
        &mut self,
        op_id: TransferId,
        content: &str,
        meta: RemoteTextMeta,
        cx: &mut Context<AppShell>,
    ) -> bool {
        self.with_view(cx, |view, cx| {
            view.diff_handle_remote_read(op_id, content, meta, cx)
        })
        .unwrap_or(false)
    }

    /// 转发远端写回完成事件到对比实体。
    pub(crate) fn handle_remote_written(
        &mut self,
        op_id: TransferId,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool {
        self.with_view(cx, |view, cx| {
            view.diff_handle_remote_written(op_id, meta, window, cx)
        })
        .unwrap_or(false)
    }

    /// 转发远端失败事件到对比实体。
    pub(crate) fn handle_remote_failure(
        &mut self,
        op_id: TransferId,
        message: &str,
        conflict: bool,
        cx: &mut Context<AppShell>,
    ) -> bool {
        self.with_view(cx, |view, cx| {
            view.diff_handle_remote_failure(op_id, message, conflict, cx)
        })
        .unwrap_or(false)
    }

    /// 对比工具栏切换到上一处或下一处差异。
    pub(crate) fn step_hunk(&mut self, forward: bool, cx: &mut Context<AppShell>) {
        self.with_view(cx, |view, cx| view.step_diff_hunk(forward, cx));
    }

    /// 请求按指定方向合并。
    pub(crate) fn request_merge(&mut self, direction: MergeDirection, cx: &mut Context<AppShell>) {
        self.with_view(cx, |view, cx| view.request_diff_merge(direction, cx));
    }

    /// 取消合并确认。
    pub(crate) fn cancel_merge(&mut self, cx: &mut Context<AppShell>) {
        self.with_view(cx, |view, cx| view.cancel_diff_merge(cx));
    }

    /// 确认执行合并。
    pub(crate) fn confirm_merge(&mut self, cx: &mut Context<AppShell>) {
        self.with_view(cx, |view, cx| view.confirm_diff_merge(cx));
    }

    /// 选择参与对比的本地文件；取消或校验失败时写状态栏并返回 `None`。
    fn pick_local_diff_file(
        &mut self,
        remote_name: &str,
        cx: &mut Context<AppShell>,
    ) -> Option<PathBuf> {
        let local_path = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("diff.pick_local_title", name = remote_name).to_string())
            .pick_file()?;
        match std::fs::metadata(&local_path) {
            Ok(meta) if meta.len() > MAX_DIFF_BYTES => {
                *self.status = Some(
                    rust_i18n::t!(
                        "diff_viewer.error_too_large",
                        limit = MAX_DIFF_BYTES,
                        path = local_path.display().to_string()
                    )
                    .to_string(),
                );
            }
            Ok(meta) if !meta.is_file() => {
                *self.status = Some(
                    rust_i18n::t!(
                        "diff_viewer.error_not_file",
                        path = local_path.display().to_string()
                    )
                    .to_string(),
                );
            }
            Err(error) => {
                *self.status = Some(
                    rust_i18n::t!(
                        "diff_viewer.error_metadata",
                        path = local_path.display().to_string(),
                        error = error.to_string()
                    )
                    .to_string(),
                );
            }
            Ok(_) => return Some(local_path),
        }
        cx.notify();
        None
    }

    /// 创建对比实体并订阅其状态事件；同一时刻只保留一个对比面板。
    fn install(
        &mut self,
        viewer: DiffViewerState,
        cx: &mut Context<AppShell>,
    ) -> Entity<DiffViewerView> {
        let command_tx = self.command_tx.clone();
        let editors = self.editors.clone();
        let view = cx.new(|_| DiffViewerView::new(viewer, command_tx, editors));
        *self.diff_viewer = Some(view.clone());
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
                    } => {
                        update_remote_entry(
                            &shell.state.workspace,
                            *tab_id,
                            path,
                            *size,
                            *modified,
                            cx,
                        );
                        cx.notify();
                    }
                },
            );
        *self.diff_viewer_subscription = Some(subscription);
        view
    }

    /// 在对比实体存在时执行一次工具栏或事件动作。
    fn with_view<R>(
        &mut self,
        cx: &mut Context<AppShell>,
        apply: impl FnOnce(&mut DiffViewerView, &mut Context<DiffViewerView>) -> R,
    ) -> Option<R> {
        let view = self.diff_viewer.clone()?;
        let result = view.update(cx, |view, cx| apply(view, cx));
        cx.notify();
        Some(result)
    }

    fn send_command(&mut self, command: impl Into<AppCommand>) -> bool {
        match self.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                self.pending_notices
                    .push(crate::app::command_send_error_text(error));
                false
            }
        }
    }
}

/// 按事件数据补丁更新 SFTP 远端条目，不读取对比或外壳实体。
fn update_remote_entry(
    workspace: &WorkspaceState,
    tab_id: TabId,
    path: &str,
    size: u64,
    modified: Option<u32>,
    cx: &mut Context<AppShell>,
) {
    if let Some(view) = workspace.tab(tab_id).and_then(|tab| tab.sftp_view()) {
        view.update(cx, |panel, _| {
            if let Some(entry) = panel
                .state
                .remote
                .entries
                .iter_mut()
                .find(|entry| entry.path == path)
            {
                entry.size = size;
                entry.modified = modified;
            }
        });
    }
}
