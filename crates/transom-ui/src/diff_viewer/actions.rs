//! 对比面板的外壳动作：打开/关闭、事件转发与工具栏适配。
//!
//! 状态与渲染归 [`super::DiffViewerView`]；这些方法需要外壳定位 SFTP 面板、
//! 订阅任务与命令通道，因此暂留 `impl AppShell`。
use super::*;

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
