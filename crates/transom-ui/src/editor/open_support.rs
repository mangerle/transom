//! 远端文本读取投递与面板种子推导。

use super::open::{PanelSeed, RemoteTextReadParams};
use super::panel::EditorPhase;
use crate::app::AppShell;
use crate::app::app_settings;
use gpui_kit::*;
use transom_core::{RemoteTextMeta, SftpCommand, SftpOperation};

impl AppShell {
    /// 投递 `ReadText`：读取上限取投递时的当前设置，失败按任务标识回填面板。
    ///
    /// 设计原理（Why）：三处读取入口只差路径与任务标识，装配与投递失败兜底
    /// 完全一致；集中在一处，避免任何一处漏掉 `fail_editor_panel_by_op` 而让
    /// 面板永远停在读取态。
    pub(crate) fn request_remote_text_read(
        &mut self,
        params: RemoteTextReadParams,
        cx: &mut Context<Self>,
    ) {
        let op_id = params.op_id;
        let max_bytes = app_settings(cx).sftp.edit_max_bytes;
        if self.send_editor_read(params, max_bytes).is_err() {
            self.fail_editor_panel_by_op(
                op_id,
                rust_i18n::t!("editor.error_read_dispatch").to_string(),
                cx,
            );
        }
    }

    /// 单纯的读取命令投递：实体回调直接调用它，失败结果由实体自己落定。
    pub(super) fn send_editor_read(
        &mut self,
        params: RemoteTextReadParams,
        max_bytes: u64,
    ) -> Result<(), ()> {
        if self.state.send_command(SftpCommand::Sftp {
            tab_id: params.tab_id,
            connection_id: params.connection_id,
            op_id: params.op_id,
            op: SftpOperation::ReadText {
                path: params.remote_path,
                max_bytes,
            },
        }) {
            Ok(())
        } else {
            Err(())
        }
    }
}

/// 由面板种子推导装载态、阶段、待处理操作与远端元信息。
pub(super) fn panel_seed_state(
    seed: PanelSeed,
) -> (
    bool,
    EditorPhase,
    Option<transom_core::TransferId>,
    Option<RemoteTextMeta>,
    bool,
) {
    match seed {
        PanelSeed::Reading { op_id, .. } => (false, EditorPhase::Reading, Some(op_id), None, false),
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
    }
}
