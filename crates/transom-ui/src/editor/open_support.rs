//! 面板种子推导（装载态、阶段、待处理操作与远端元信息）。

use super::open::PanelSeed;
use super::panel::EditorPhase;
use transom_core::RemoteTextMeta;

/// 由面板种子推导装载态、阶段、待处理操作与远端元信息。
pub(crate) fn panel_seed_state(
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
