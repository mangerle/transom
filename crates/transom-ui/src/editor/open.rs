//! SFTP 编辑面板的打开与读取生命周期：整份拉取远端文本、装载缓冲区、
//! 失败重试与冲突后的重新载入。
//!
//! 面板状态与窗体动作在 `panel.rs` / `actions.rs`，保存回传在 `ops.rs`，
//! 关闭保护在 `close.rs`，渲染在 `view.rs`。
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
//! - 实体化边界：面板装配、读取结果装载与重读状态都在 `EditorPanels`；外壳只
//!   发读取命令并在投递失败时把错误交回实体。

use super::entity::EditorPanels;
use super::open_support::panel_seed_state;
use super::panel::{
    EditorPanelState, EditorPhase, PANEL_HEIGHT, PANEL_WIDTH, PendingGoto, caret_of,
    cascade_origin, editor_language_token,
};
use crate::app::AppShell;
use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteTextMeta, SftpCommand, SftpOperation, TabId, TransferId};

mod panel;

pub(crate) use self::panel::RemoteEditorOpen;

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
        connection_id: ConnectionId,
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
pub(crate) enum PanelSeed {
    /// 打开既有文件：面板先出现，等待 `SftpTextRead` 装载内容。
    Reading {
        op_id: TransferId,
        permissions: Option<u32>,
    },
    /// 新建文件：空缓冲区、基线为空，首次保存创建远端文件。
    NewFile,
}

/// 建立面板时需要从设置页带进来的编辑器外观选项。
///
/// 由外壳读设置后传入：实体不直接读工作区，避免渲染与状态边界纠缠。
pub(crate) struct EditorPanelSettings {
    pub(crate) line_numbers: bool,
    pub(crate) soft_wrap: bool,
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

/// 建立编辑面板的输入参数：文件归属、路径与初始形态。
///
/// 参数对象化：编辑器装配同时需要标签、连接、路径、文件名与读取/新建形态，
/// 平铺成参数列表既难读也容易把连接与路径配错。
pub(crate) struct SpawnEditorPanelParams {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    pub(crate) seed: PanelSeed,
    /// 打开后一次性消费的行列定位；终端代码链接用它把光标送到目标行。
    pub(crate) pending_goto: PendingGoto,
}

/// 远端文本读取请求：列表打开、终端代码链接与失败重读共用同一条读取通道。
///
/// 参数对象化：四类信息里有三个是"目标"、一个是面板登记的任务标识，
/// 平铺成参数列表容易把连接与路径配错。
#[derive(Clone)]
pub(crate) struct RemoteTextReadParams {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_path: String,
    /// 面板读取态登记的任务标识；投递失败时按它回填错误。
    pub(crate) op_id: TransferId,
}

impl EditorPanels {
    /// 同一文件已在编辑中：把它抬到最上层并返回编辑器实体，调用方负责聚焦或
    /// 按代码链接定位；没有同文件面板时返回 `None`。
    pub(crate) fn raise_existing(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        remote_path: &str,
    ) -> Option<Entity<EditorState>> {
        let index = self
            .manager()
            .panels
            .iter()
            .position(|item| item.same_file(tab_id, connection_id, remote_path))?;
        let item = self.manager_mut().panels.remove(index);
        let editor = item.editor.clone();
        self.manager_mut().panels.push(item);
        Some(editor)
    }

    /// 建立编辑面板：空缓冲区、订阅就位、按已开面板数量级联落位。
    ///
    /// 读取态与新建态共用一套装配，只差初始状态字段（见 `PanelSeed`）；
    /// 新建面板可编辑、无在途任务，读取面板等待 `SftpTextRead` 装载内容。
    /// 返回读取请求：只有读取态需要外壳投递 `ReadText`。
    /// 编辑器输入与光标订阅：标记脏状态、刷新变更装饰、同步光标并避免自激重绘。
    fn subscribe_editor(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> (Subscription, Subscription) {
        let change_sub = cx.subscribe(editor, |this, editor, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let mut schedule_decorations = false;
            if let Some(item) = this
                .manager_mut()
                .panels
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
                schedule_decorations = true;
            }
            if schedule_decorations {
                this.schedule_change_decorations(&editor, cx);
            }
            cx.notify();
        });
        // 光标移动只 notify、不 emit 事件，因此观察通知本身；只在行列真的变了时
        // 才重绘，既让状态栏实时，也切断「面板重绘 → 编辑器通知 → 面板重绘」的自激。
        let caret_sub = cx.observe(editor, |this, editor, cx| {
            let next = caret_of(&editor, cx);
            let changed = this
                .manager_mut()
                .panels
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
        (change_sub, caret_sub)
    }

    /// 按权限设置只读、聚焦并读取初始光标位置。
    fn prepare_editor_state(
        &self,
        editor: &Entity<EditorState>,
        seed: &PanelSeed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<u32>, bool, (u32, u32)) {
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
        let caret = caret_of(editor, cx);
        (permissions, writable, caret)
    }

    pub(crate) fn spawn_editor_panel(
        &mut self,
        params: SpawnEditorPanelParams,
        settings: EditorPanelSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<RemoteTextReadParams> {
        let SpawnEditorPanelParams {
            tab_id,
            connection_id,
            remote_path,
            file_name,
            seed,
            pending_goto,
        } = params;
        let language = editor_language_token(&remote_path);
        // 软换行与行号跟随设置页选择：日志类内容常关行号开换行，
        // 配置文件则相反，属于高频个性化项。
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.clone())
                .line_number(settings.line_numbers)
                .folding(true)
                .searchable(true)
                .soft_wrap(settings.soft_wrap)
        });
        let (gutter_decorations, overview_ruler_decorations) = editor.update(cx, |state, cx| {
            let gutter = state.create_gutter_decorations_collection(Vec::new(), cx);
            let overview = state.create_overview_ruler_decorations_collection(Vec::new(), cx);
            (gutter, overview)
        });
        let (permissions, writable, caret) = self.prepare_editor_state(&editor, &seed, window, cx);
        let cascade = self.len();
        let viewport = window.viewport_size();
        let (left, top) = cascade_origin(cascade, viewport);
        let (change_sub, caret_sub) = self.subscribe_editor(&editor, cx);
        let (loaded, phase, pending_op, remote_meta, new_file) = panel_seed_state(seed);
        self.manager_mut().panels.push(EditorPanelState {
            tab_id,
            connection_id,
            remote_path: remote_path.clone(),
            file_name,
            language,
            editor,
            gutter_decorations,
            overview_ruler_decorations,
            decoration_revision: 0,
            decoration_task: None,
            baseline: SharedString::default(),
            remote_meta,
            new_file,
            pending_goto,
            pending_sudo_probe: None,
            sudo_probe_attempted: false,
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
            diff_options: crate::diff_pane::DiffViewOptions::default(),
            diff_computing: false,
            diff_error: None,
            diff_compute_token: 0,
            diff_additions: 0,
            diff_deletions: 0,
            diff_prepare_task: None,
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
        match phase {
            EditorPhase::Reading => Some(RemoteTextReadParams {
                tab_id,
                connection_id,
                remote_path,
                op_id: pending_op?,
            }),
            _ => None,
        }
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
        let Some(index) =
            self.manager().panels.iter().position(|item| {
                item.pending_op == Some(op_id) && item.phase == EditorPhase::Reading
            })
        else {
            return;
        };
        let item = &mut self.manager_mut().panels[index];
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
        item.clear_diff_preview();
        // GBK 等转码内容只读预览：转码回写需要反向编码决策，超出内置编辑器
        // 的安全边界；缓冲区不可编辑，dirty 不会置位，保存自然被禁用。
        if meta.transcoded {
            item.writable = false;
            item.editor
                .update(cx, |state, cx| state.set_readonly(true, cx));
        }
        // 终端代码链接带进来的定位请求在内容落地后一次性消费；普通打开仍聚焦到
        // 上次位置之外的行首，两种来源共用同一个跳转 helper，边界口径一致。
        let goto = item.pending_goto.take();
        let editor = item.editor.clone();
        if let Some((line, column)) = goto {
            super::navigate::apply_editor_goto(&editor, line, column, window, cx);
        } else {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
        self.clear_change_decorations(&editor, cx);
        cx.notify();
    }

    /// 重发读取命令：读取失败后的"重试读取"与冲突横幅的"放弃本地并重新载入"
    /// 共用同一条装载路径。
    ///
    /// 面板回到 `Reading` 态，主体区被读取占位替换——既给出进行中状态，也避免
    /// 用户继续在即将被整体覆盖的旧内容上打字。缓冲区旧内容保持到新内容抵达，
    /// 重新载入失败时旧内容与脏标记原样保留。
    pub(crate) fn redrive_editor_read(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some((tab_id, connection_id, remote_path, can_retry)) =
            self.map_panel(editor, |item| {
                // 读取或回传在途不接受重复触发：连点重试只保留第一次。
                let can_retry = item.phase != EditorPhase::Reading && item.pending_op.is_none();
                (
                    item.tab_id,
                    item.connection_id,
                    item.remote_path.clone(),
                    can_retry,
                )
            })
        else {
            return;
        };
        if !can_retry {
            return;
        }
        let op_id = TransferId::new();
        self.update_panel(editor, |item| {
            item.phase = EditorPhase::Reading;
            item.pending_op = Some(op_id);
            item.error = None;
            item.conflict = false;
            // 重新载入会整体替换缓冲区，差异预览展示的旧基线随之失效。
            item.clear_diff_preview();
        });
        let request = RemoteTextReadParams {
            tab_id,
            connection_id,
            remote_path,
            op_id,
        };
        let max_bytes = self.edit_max_bytes(cx);
        let sent = self.command_tx().is_some_and(|tx| {
            tx.try_send(
                SftpCommand::Sftp {
                    tab_id: request.tab_id,
                    connection_id: request.connection_id,
                    op_id: request.op_id,
                    op: SftpOperation::ReadText {
                        path: request.remote_path,
                        max_bytes,
                    },
                }
                .into(),
            )
            .is_ok()
        });
        if !sent {
            self.fail_by_op(
                op_id,
                rust_i18n::t!("editor.error_read_dispatch").to_string(),
                cx,
            );
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
