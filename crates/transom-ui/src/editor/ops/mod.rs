//! SFTP 编辑面板的保存回传生命周期：整份缓冲区写回远端、版本指纹冲突的
//! 拦截与回填、以及结果事件的落地。
//!
//! 打开与读取在 `open`，面板状态与窗体动作在 `panel` / `actions`，
//! 关闭保护在 `close`，渲染在 `view`。

use super::entity::EditorPanels;
use super::panel::EditorPhase;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{
    ConnectionId, ErrorCode, RemoteTextMeta, SftpCommand, SftpOperation, SudoCapability,
    SudoUnavailable, TabId, TransferId,
};

mod result;
pub(in crate::editor) use result::{
    EditorFailureAction, EditorWrittenAction, SudoProbeOutcome,
};
mod save;
pub(in crate::editor) use save::{
    EditorWriteMode, EditorWriteRequest, SudoPasswordRequest, SudoProbeRequest,
};

/// 把提权不可用原因翻译成用户能读懂、可据以采取行动的说明。
fn sudo_unavailable_text(reason: SudoUnavailable) -> String {
    match reason {
        SudoUnavailable::NotInstalled => rust_i18n::t!("editor.sudo_not_installed").to_string(),
        SudoUnavailable::RequiresTty => rust_i18n::t!("editor.sudo_requires_tty").to_string(),
        SudoUnavailable::NotAuthorized => rust_i18n::t!("editor.sudo_not_authorized").to_string(),
        SudoUnavailable::Other => rust_i18n::t!("editor.sudo_probe_failed_reason").to_string(),
    }
}
