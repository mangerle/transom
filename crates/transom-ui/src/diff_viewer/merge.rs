//! 单向合并：确认、写回远端或本地、以及写回结果落地。

use super::local_io::write_local_text;
use super::{DiffConfirm, DiffPhase};
use crate::app::AppShell;
use crate::editor::EditorPhase;
use gpui_kit::*;
use transom_core::{
    ConnectionId, MergeDirection, RemoteTextMeta, SftpCommand, SftpOperation, TabId, TransferId,
};

struct MergePlan {
    tab_id: TabId,
    connection_id: Option<ConnectionId>,
    remote_path: String,
    expected: Option<RemoteTextMeta>,
    direction: MergeDirection,
    merged: String,
}

impl AppShell {
    /// 请求合并当前变更块：先进入确认态，真正写回在用户确认后才发生。
    pub(crate) fn request_diff_merge(&mut self, direction: MergeDirection, cx: &mut Context<Self>) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        if viewer.phase != DiffPhase::Ready {
            return;
        }
        let Some(hunk) = viewer.hunk_index else {
            return;
        };
        if direction == MergeDirection::LeftToRight
            && let Err(reason) = viewer.remote_write_blocker()
        {
            viewer.error = Some(reason.to_string());
            cx.notify();
            return;
        }
        viewer.confirm = Some(DiffConfirm { direction, hunk });
        cx.notify();
    }

    pub(crate) fn cancel_diff_merge(&mut self, cx: &mut Context<Self>) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            viewer.confirm = None;
        }
        cx.notify();
    }

    /// 确认合并：把选中的变更块从源侧应用到目标侧。
    pub(crate) fn confirm_diff_merge(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.build_merge_plan() else {
            return;
        };
        match plan.direction {
            MergeDirection::LeftToRight => self.submit_diff_remote_write(plan, cx),
            MergeDirection::RightToLeft => self.submit_diff_local_write(plan.merged, cx),
        }
    }

    fn build_merge_plan(&mut self) -> Option<MergePlan> {
        let viewer = self.diff_viewer.as_mut()?;
        let confirm = viewer.confirm.take()?;
        let diff = viewer.diff.as_ref()?;
        let local = viewer.local_text.as_deref()?;
        let remote = viewer.remote_text.as_deref()?;
        let merged = diff.apply_hunk(local, remote, confirm.hunk, confirm.direction);
        let Some(merged) = merged else {
            viewer.error = Some("变更块已失效，请重新对比".to_string());
            return None;
        };
        if confirm.direction == MergeDirection::LeftToRight
            && let Err(reason) = viewer.remote_write_blocker()
        {
            viewer.error = Some(reason.to_string());
            return None;
        }
        Some(MergePlan {
            tab_id: viewer.tab_id,
            connection_id: viewer.connection_id,
            remote_path: viewer.remote_path.clone(),
            expected: viewer.remote_meta,
            direction: confirm.direction,
            merged,
        })
    }

    fn submit_diff_remote_write(&mut self, plan: MergePlan, cx: &mut Context<Self>) {
        let Some(connection_id) = plan.connection_id else {
            return;
        };
        let backup = self.settings.sftp.editor_write_backup;
        let op_id = TransferId::new();
        let merged = plan.merged;
        let sent = self.send_command(SftpCommand::Sftp {
            tab_id: plan.tab_id,
            connection_id,
            op_id,
            op: SftpOperation::WriteText {
                path: plan.remote_path,
                content: merged.clone().into_bytes(),
                expected: plan.expected,
                force: false,
                backup,
            },
        });
        if let Some(viewer) = self.diff_viewer.as_mut() {
            if sent {
                viewer.pending_write = Some(op_id);
                viewer.pending_merged = Some(merged);
                viewer.phase = DiffPhase::Writing;
                viewer.error = None;
            } else {
                viewer.error = Some("写入远端文件的命令投递失败".to_string());
            }
        }
        cx.notify();
    }

    fn submit_diff_local_write(&mut self, merged: String, cx: &mut Context<Self>) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        let path = viewer.local_path.clone();
        let Some(expected) = viewer.local_text.clone() else {
            return;
        };
        viewer.pending_local_merged = Some(merged.clone());
        viewer.phase = DiffPhase::Writing;
        viewer.error = None;
        cx.spawn(async move |this, cx| {
            let result = crate::sftp::spawn_blocking_local(move || {
                write_local_text(&path, &expected, &merged)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_diff_local_write(result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_diff_local_write(
        &mut self,
        result: Option<Result<(), String>>,
        cx: &mut Context<Self>,
    ) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        match result {
            Some(Ok(())) => {
                if let Some(text) = viewer.pending_local_merged.take() {
                    viewer.local_text = Some(text);
                }
                viewer.error = None;
                viewer.phase = DiffPhase::Ready;
            }
            Some(Err(error)) => {
                viewer.pending_local_merged = None;
                viewer.phase = DiffPhase::Ready;
                viewer.error = Some(error);
            }
            None => {
                viewer.pending_local_merged = None;
                viewer.phase = DiffPhase::Ready;
                viewer.error = Some("本地写入线程启动失败".to_string());
            }
        }
        self.recompute_diff(cx);
        cx.notify();
    }

    /// 远端读取完成：命中对比面板的在途读取时消费，否则交回编辑器处理。
    pub(crate) fn diff_handle_remote_read(
        &mut self,
        op_id: TransferId,
        content: &str,
        meta: RemoteTextMeta,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return false;
        };
        if viewer.pending_read != Some(op_id) {
            return false;
        }
        viewer.pending_read = None;
        viewer.remote_text = Some(content.to_string());
        viewer.remote_meta = Some(meta);
        viewer.phase = DiffPhase::Ready;
        self.recompute_diff(cx);
        cx.notify();
        true
    }

    /// 远端写入完成：更新远端内容与指纹，并把已打开的编辑器基线同步到新版本。
    pub(crate) fn diff_handle_remote_written(
        &mut self,
        op_id: TransferId,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return false;
        };
        if viewer.pending_write != Some(op_id) {
            return false;
        }
        let Some(text) = viewer.pending_merged.take() else {
            return true;
        };
        viewer.pending_write = None;
        viewer.remote_meta = Some(meta);
        viewer.remote_text = Some(text.clone());
        let tab_id = viewer.tab_id;
        let remote_path = viewer.remote_path.clone();
        if let Some(tab) = self.workspace.tab_mut(tab_id)
            && let Some(panel) = tab.sftp_mut()
            && let Some(entry) = panel
                .remote
                .entries
                .iter_mut()
                .find(|entry| entry.path == remote_path)
        {
            entry.size = meta.size;
            entry.modified = meta.mtime;
        }
        self.sync_editor_after_diff_write(tab_id, &remote_path, &text, meta, window, cx);
        self.recompute_diff(cx);
        cx.notify();
        true
    }

    fn sync_editor_after_diff_write(
        &mut self,
        tab_id: TabId,
        remote_path: &str,
        text: &str,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editors
            .panels
            .iter()
            .position(|item| item.same_file(tab_id, remote_path))
        else {
            return;
        };
        let item = &mut self.editors.panels[index];
        item.baseline = SharedString::from(text.to_string());
        item.remote_meta = Some(meta);
        // 合并已成功落盘，编辑器上旧的冲突/失败横幅针对的是过期远端状态。
        item.conflict = false;
        item.error = None;
        if item.phase == EditorPhase::Failed {
            item.phase = EditorPhase::Idle;
        }
        // 未改动的编辑器缓冲区直接跟随远端新内容；有未保存修改时保留用户输入，
        // 下一次保存会以新指纹为基准，而不是把这次合并当成冲突。
        // 远端基线变了，编辑器里正在展示的"保存前 diff"随即过期。
        item.showing_diff = false;
        item.diff = None;
        if !item.dirty {
            item.editor
                .update(cx, |state, cx| state.set_value(text, window, cx));
            item.caret = crate::editor::caret_of(&item.editor, cx);
            item.written_size = Some(meta.size);
        }
    }

    /// 远端读取或写回失败：按在途操作归属更新对比面板，不触碰编辑器状态。
    ///
    /// 读取失败与写回失败共用 `SftpTextFailed` 事件，必须同时在两处等待位上
    /// 判断归属；只看写回会让读取失败的对比面板永远停在加载态。
    pub(crate) fn diff_handle_remote_failure(
        &mut self,
        op_id: TransferId,
        message: &str,
        conflict: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return false;
        };
        if viewer.pending_read == Some(op_id) {
            viewer.pending_read = None;
            viewer.phase = DiffPhase::Ready;
            viewer.error = Some(format!("读取远端文件失败：{message}"));
            cx.notify();
            return true;
        }
        if viewer.pending_write != Some(op_id) {
            return false;
        }
        viewer.pending_write = None;
        viewer.pending_merged = None;
        viewer.phase = DiffPhase::Ready;
        viewer.error = Some(if conflict {
            format!("远端已被修改，本次合并未写入：{message}")
        } else {
            message.to_string()
        });
        cx.notify();
        true
    }
}
