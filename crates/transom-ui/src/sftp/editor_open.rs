//! SFTP 编辑面板的打开与读取生命周期：整份拉取远端文本、装载缓冲区、
//! 失败重试与冲突后的重新载入。
//!
//! 面板状态与窗体动作在 `editor_panel.rs`，保存回传在 `editor_ops.rs`，
//! 关闭保护在 `editor_close.rs`，渲染在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 读写走 `SftpOperation::ReadText` / `WriteText` 独立通道，**不进入传输队列**。
//!   编辑取文件是应用内部行为，混进用户队列会带来三类真问题：占掉
//!   `max_concurrent_transfers` 的并发槽拖慢用户真正的批量传输、用户可以取消或清空
//!   掉在途的编辑读写（表现为编辑器莫名打不开、回传结果无人认领）、以及完成统计把
//!   编辑算进"传输完成"。
//! - 因此也不再落临时文件：内容只存在于内存缓冲区。代价是崩溃不恢复、没有 SHA256
//!   端到端校验；换来的是 `/tmp` 不再留明文副本（这类远端文件常含口令），并少一次
//!   本地读写。读取时顺带回传版本指纹（字节数 + mtime），保存前据此做并发冲突比对。
//! - 面板先于读取结果出现，处于 `Reading` 态：用户双击后立刻得到反馈，比"等整份
//!   传输完成才弹窗"更符合编辑器的交互预期。

use super::editor_panel::{
    EditorPanelState, EditorPhase, PANEL_HEIGHT, PANEL_WIDTH, caret_of, cascade_origin,
    editor_language_token,
};
use super::format_size;
use crate::app::AppShell;
use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::*;
use transom_core::{AppCommand, ConnectionId, RemoteTextMeta, SftpOperation, TabId, TransferId};

/// 编辑入口的准入判定结果。
///
/// 把"能不能开"与"怎么报错"分开：判定只读一次目录项，报错统一落到该栏错误位，
/// 避免一条提示语为了冒泡而把整条调用链改成 `Result`。
enum EditGate {
    /// 目录项缺失（列表已刷新或路径不合法）。
    Missing,
    /// 目标是目录。
    IsDir,
    /// 超过内置编辑器上限。
    TooLarge { name: String, size: u64 },
    /// 允许打开。
    Allowed {
        connection_id: Option<ConnectionId>,
        name: String,
    },
}

impl AppShell {
    /// 双击/点击远端文本文件：立刻开一个读取态面板，并发命令整份拉取内容。
    pub(crate) fn sftp_edit_remote_file(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let max_bytes = self.settings.sftp.edit_max_bytes;
        let gate = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| {
                let panel = tab.sftp.as_ref()?;
                let entry = panel.entries.iter().find(|item| item.path == remote_path)?;
                if entry.is_dir {
                    return Some(EditGate::IsDir);
                }
                if entry.size > max_bytes {
                    return Some(EditGate::TooLarge {
                        name: entry.name.clone(),
                        size: entry.size,
                    });
                }
                Some(EditGate::Allowed {
                    connection_id: tab.connection_id,
                    name: entry.name.clone(),
                })
            })
            .unwrap_or(EditGate::Missing);

        let (connection_id, file_name) = match gate {
            EditGate::Missing => {
                self.editor_reject(tab_id, rust_i18n::t!("sftp.select_file").to_string(), cx);
                return;
            }
            EditGate::IsDir => return,
            EditGate::TooLarge { name, size } => {
                self.editor_reject(
                    tab_id,
                    format!(
                        "{name}（{}）超过内置编辑器上限 {}：请在终端窗格用编辑器打开，或先下载到本地再编辑",
                        format_size(size),
                        format_size(max_bytes)
                    ),
                    cx,
                );
                return;
            }
            EditGate::Allowed {
                connection_id,
                name,
            } => (connection_id, name),
        };
        let Some(connection_id) = connection_id else {
            self.editor_reject(tab_id, "SSH 连接尚未就绪，无法编辑远端文件".to_string(), cx);
            return;
        };

        // 同一文件已在编辑中就不再重复取文件：抬到最前并聚焦编辑器。
        if let Some(index) = self
            .editor_panels
            .iter()
            .position(|item| item.same_file(tab_id, &remote_path))
        {
            let item = self.editor_panels.remove(index);
            let editor = item.editor.clone();
            self.editor_panels.push(item);
            editor.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        // 这里的尺寸闸门只是第一道：目录列表的尺寸可能过期，net 层还会在读取循环内
        // 按同一上限二次设限，两道一起才挡得住"列出之后文件变大"的情况。
        let op_id = TransferId::new();
        self.spawn_editor_panel(tab_id, remote_path.clone(), file_name, op_id, window, cx);
        if let Err(err) = self.services.command_tx.send(AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes,
            },
        }) {
            self.fail_editor_panel_by_op(op_id, format!("读取命令投递失败: {err}"), cx);
        }
    }

    /// 建立一个处于读取态的面板：空缓冲区、订阅就位、按已开面板数量级联落位。
    fn spawn_editor_panel(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        file_name: String,
        op_id: TransferId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = editor_language_token(&remote_path);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.clone())
                .line_number(true)
                .folding(true)
                .searchable(true)
                .soft_wrap(true)
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        let caret = caret_of(&editor, cx);
        let cascade = self.editor_panels.len();
        let viewport = window.viewport_size();
        let (left, top) = cascade_origin(cascade, viewport);
        let change_sub = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            if let Some(item) = this
                .editor_panels
                .iter_mut()
                .find(|item| item.editor == editor)
            {
                // 未成功装载远端内容的缓冲区不允许变成"可保存的改动"，
                // 否则读取失败后随手打几个字就能把远端文件覆盖掉。
                if !item.loaded {
                    return;
                }
                item.dirty = true;
                item.phase = EditorPhase::Idle;
                item.written_size = None;
                item.error = None;
            }
            cx.notify();
        });
        // 光标移动只 notify、不 emit 事件，因此观察通知本身；只在行列真的变了时
        // 才重绘，既让状态栏实时，也切断「面板重绘 → 编辑器通知 → 面板重绘」的自激。
        let caret_sub = cx.observe(&editor, |this, editor, cx| {
            let next = caret_of(&editor, cx);
            let changed = this
                .editor_panels
                .iter_mut()
                .find(|item| item.editor == editor)
                .is_some_and(|item| {
                    if item.caret == next {
                        false
                    } else {
                        item.caret = next;
                        true
                    }
                });
            if changed {
                cx.notify();
            }
        });
        self.editor_panels.push(EditorPanelState {
            tab_id,
            remote_path,
            file_name,
            language,
            editor,
            baseline: SharedString::default(),
            remote_meta: None,
            caret,
            dirty: false,
            loaded: false,
            phase: EditorPhase::Reading,
            written_size: None,
            pending_op: Some(op_id),
            error: None,
            conflict: false,
            confirm: None,
            collapsed: false,
            maximized: false,
            left,
            top,
            width: PANEL_WIDTH,
            height: PANEL_HEIGHT,
            drag_grab: None,
            _change_sub: change_sub,
            _caret_sub: caret_sub,
        });
        cx.notify();
    }

    /// 远端文本读回来了：填充缓冲区并切到可编辑态，同时缓存版本指纹。
    ///
    /// `set_value` 在库内显式关闭了事件外发并重置了撤销栈，因此装载远端内容不会被
    /// `Change` 订阅误判成用户改动；脏基准仍在这一处整体重取，两条路径口径一致。
    pub(crate) fn apply_editor_text(
        &mut self,
        op_id: TransferId,
        content: String,
        meta: RemoteTextMeta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editor_panels
            .iter()
            .position(|item| item.pending_op == Some(op_id) && item.phase == EditorPhase::Reading)
        else {
            return;
        };
        let item = &mut self.editor_panels[index];
        item.editor
            .update(cx, |state, cx| state.set_value(content, window, cx));
        item.baseline = item.editor.read(cx).value();
        item.remote_meta = Some(meta);
        item.caret = caret_of(&item.editor, cx);
        item.dirty = false;
        item.loaded = true;
        item.phase = EditorPhase::Idle;
        item.pending_op = None;
        item.error = None;
        item.conflict = false;
        item.editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// 重发读取命令：读取失败后的"重试读取"与冲突横幅的"放弃本地并重新载入"
    /// 共用同一条装载路径。
    ///
    /// 面板回到 `Reading` 态，主体区被读取占位替换——既给出进行中状态，也避免
    /// 用户继续在即将被整体覆盖的旧内容上打字。缓冲区旧内容保持到新内容抵达，
    /// 重新载人失败时旧内容与脏标记原样保留。
    pub(crate) fn redrive_editor_read(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .editor_panels
            .iter()
            .find(|item| item.editor == *editor)
        else {
            return;
        };
        // 读取或回传在途不接受重复触发：连点重试只保留第一次。
        if matches!(item.phase, EditorPhase::Reading) || item.pending_op.is_some() {
            return;
        }
        let tab_id = item.tab_id;
        let remote_path = item.remote_path.clone();
        let Some(connection_id) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.connection_id)
        else {
            if let Some(item) = self.editor_panel_mut(editor) {
                item.phase = EditorPhase::Failed;
                item.error = Some("SSH 连接尚未就绪，无法读取远端文件".to_string());
            }
            cx.notify();
            return;
        };
        let op_id = TransferId::new();
        if let Some(item) = self.editor_panel_mut(editor) {
            item.phase = EditorPhase::Reading;
            item.pending_op = Some(op_id);
            item.error = None;
            item.conflict = false;
        }
        let max_bytes = self.settings.sftp.edit_max_bytes;
        if let Err(err) = self.services.command_tx.send(AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes,
            },
        }) {
            self.fail_editor_panel_by_op(op_id, format!("读取命令投递失败: {err}"), cx);
            return;
        }
        cx.notify();
    }
}
