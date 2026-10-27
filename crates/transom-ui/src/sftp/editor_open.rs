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
use transom_core::{ConnectionId, RemoteTextMeta, SftpCommand, SftpOperation, TabId, TransferId};

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
        /// 目录项快照的权限位；`None` 表示服务器未回传，视为可写。
        permissions: Option<u32>,
    },
}

/// 按权限位推定文件可写：所有写位皆无（如 444）时不可写。
///
/// 只看权限位、不做属主比对（SFTP 拿不到本端连接用户的 uid 与文件属主的
/// 对应关系）：属主文件仍可正常编辑，root 属主但 644 的文件会放行到保存时
/// 由远端报权限错误兜底，属于已知的推定局限。
fn permissions_writable(permissions: Option<u32>) -> bool {
    permissions.is_none_or(|p| p & 0o222 != 0)
}

/// 面板初始形态：普通打开（读取在途）与新建文件（空缓冲区立即可编辑）。
///
/// 权限位属于"打开既有条目"的信息（来自目录列表快照），随读取种子传入；
/// 新建文件没有已存在的权限位可依据，视作可写。
enum PanelSeed {
    /// 打开既有文件：面板先出现，等待 `SftpTextRead` 装载内容。
    Reading {
        op_id: TransferId,
        permissions: Option<u32>,
    },
    /// 新建文件：空缓冲区、基线为空，首次保存创建远端文件。
    NewFile,
}

/// 取远端路径的父目录；根目录文件返回 `/`，无 `/` 的相对路径返回 `None`。
pub(crate) fn parent_dir(path: &str) -> Option<&str> {
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    let index = trimmed.rfind('/')?;
    if index == 0 {
        Some("/")
    } else {
        Some(&trimmed[..index])
    }
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
            .workspace
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
                    permissions: entry.permissions,
                })
            })
            .unwrap_or(EditGate::Missing);

        let (connection_id, file_name, permissions) = match gate {
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
                permissions,
            } => (connection_id, name, permissions),
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
        self.spawn_editor_panel(
            tab_id,
            remote_path.clone(),
            file_name,
            PanelSeed::Reading { op_id, permissions },
            window,
            cx,
        );
        if !self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes,
            },
        }) {
            self.fail_editor_panel_by_op(op_id, "读取命令投递失败".to_string(), cx);
        }
    }

    /// 新建远端文件并打开编辑面板：空缓冲区立即可编辑，首次保存才创建远端文件。
    ///
    /// 面板携带零尺寸指纹：若同名非空文件恰好已存在（目录列表过期时可能发生），
    /// 首次保存会被 net 层指纹比对拦成冲突，弹出横幅而不是静默覆盖。
    pub(crate) fn spawn_new_editor_panel(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        file_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_editor_panel(
            tab_id,
            remote_path,
            file_name,
            PanelSeed::NewFile,
            window,
            cx,
        );
    }

    /// 建立编辑面板：空缓冲区、订阅就位、按已开面板数量级联落位。
    ///
    /// 读取态与新建态共用一套装配，只差初始状态字段（见 `PanelSeed`）；
    /// 新建面板可编辑、无在途任务，读取面板等待 `SftpTextRead` 装载内容。
    fn spawn_editor_panel(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        file_name: String,
        seed: PanelSeed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = editor_language_token(&remote_path);
        // 软换行与行号跟随设置页选择：日志类内容常关行号开换行，
        // 配置文件则相反，属于高频个性化项。
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.clone())
                .line_number(self.settings.sftp.editor_line_numbers)
                .folding(true)
                .searchable(true)
                .soft_wrap(self.settings.sftp.editor_soft_wrap)
        });
        // 无写权限的文件（如 444 的密钥、锁定的配置）直接按只读打开：
        // 用户能看到内容但编辑器拒绝输入，避免改完才发现保存必然失败。
        // 新建文件没有已存在的权限位可依据，视作可写。
        let permissions = match &seed {
            PanelSeed::Reading { permissions, .. } => *permissions,
            PanelSeed::NewFile => None,
        };
        let writable = permissions_writable(permissions);
        if !writable {
            editor.update(cx, |state, cx| state.set_readonly(true, cx));
        }
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
        let (loaded, phase, pending_op, remote_meta, new_file) = match seed {
            PanelSeed::Reading { op_id, .. } => {
                (false, EditorPhase::Reading, Some(op_id), None, false)
            }
            // 新建面板：可编辑即可打字；零尺寸指纹让首次保存具备同名覆盖保护。
            PanelSeed::NewFile => (
                true,
                EditorPhase::Idle,
                None,
                Some(RemoteTextMeta {
                    size: 0,
                    mtime: None,
                    bom: false,
                    crlf: false,
                    transcoded: false,
                }),
                true,
            ),
        };
        self.editor_panels.push(EditorPanelState {
            tab_id,
            remote_path,
            file_name,
            language,
            editor,
            baseline: SharedString::default(),
            remote_meta,
            new_file,
            caret,
            dirty: false,
            loaded,
            phase,
            written_size: None,
            pending_op,
            error: None,
            permissions,
            writable,
            close_after_write: false,
            diff: None,
            showing_diff: false,
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
        item.showing_diff = false;
        item.diff = None;
        // GBK 等转码内容只读预览：转码回写需要反向编码决策，超出内置编辑器
        // 的安全边界；缓冲区不可编辑，dirty 不会置位，保存自然被禁用。
        if meta.transcoded {
            item.writable = false;
            item.editor
                .update(cx, |state, cx| state.set_readonly(true, cx));
        }
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
            .workspace
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
            // 重新载入会整体替换缓冲区，差异预览展示的旧基线随之失效。
            item.showing_diff = false;
            item.diff = None;
        }
        let max_bytes = self.settings.sftp.edit_max_bytes;
        if !self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op: SftpOperation::ReadText {
                path: remote_path,
                max_bytes,
            },
        }) {
            self.fail_editor_panel_by_op(op_id, "读取命令投递失败".to_string(), cx);
            return;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::parent_dir;

    #[test]
    fn test_parent_dir_extracts_remote_parent() {
        assert_eq!(parent_dir("/etc/nginx/nginx.conf"), Some("/etc/nginx"));
        assert_eq!(parent_dir("/app/config.yaml"), Some("/app"));
        // 根目录下的文件：父目录是根本身，而不是空串。
        assert_eq!(parent_dir("/hosts.txt"), Some("/"));
        // 末尾斜杠不影响解析。
        assert_eq!(parent_dir("/app/config.yaml/"), Some("/app"));
        // 相对路径没有父目录语义，交由调用方按当前目录拼接。
        assert_eq!(parent_dir("config.yaml"), None);
    }
}
