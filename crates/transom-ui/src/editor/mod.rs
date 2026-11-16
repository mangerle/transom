//! 远端文本编辑器子系统。
//!
//! 设计说明（Why）：
//! - 实现初衷：从 SFTP 文件管理器中完整剥离文本编辑器的全部生命周期
//!   （文件打开、暂存、冲突检测、Diff、编码转换、行跳转与拖拽布局）；
//! - 核心优势：
//!   1. 解除 SFTP 状态机与编辑器状态机的深度耦合；
//!   2. 编辑器作为独立的浮层子系统，具备可拖拽、可折叠、非阻塞的生命周期；
//!   3. SFTP 仅作为“在编辑器中打开”的触发来源之一，职责清晰单纯。

mod actions;
mod banner;
mod bridge;
mod close;
mod code_link;
mod decorations;
mod diff;
mod diff_search;
mod drag_tracker;
mod entity;
mod list;
mod manager;
mod open;
mod ops;
mod panel;
mod status;
mod view;

/// 编辑器“跳转到行”弹窗的状态。
pub(crate) struct EditorGotoLineModalState {
    pub(crate) editor: gpui_kit::Entity<gpui_kit::component::input::EditorState>,
    pub(crate) input: gpui_kit::Entity<gpui_kit::component::input::InputState>,
    pub(crate) error: Option<String>,
    pub(crate) _enter_sub: gpui_kit::Subscription,
}

/// 把编辑器光标移到 1 基的行列位置，展开被折叠的行并收敛越界值。
pub(crate) fn apply_editor_goto(
    editor: &gpui_kit::Entity<gpui_kit::component::input::EditorState>,
    line: u32,
    column: Option<u32>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::component::input::{Position, RopeExt};
    let total = editor.read(cx).text().lines_len().max(1) as u32;
    let line = line.max(1).min(total);
    let column = column.unwrap_or(1).max(1) - 1;
    let position = Position::new(line - 1, column);
    editor.update(cx, |state, cx| {
        state.unfold_at(position, cx);
        state.set_cursor_position(position, window, cx);
    });
}

/// 由面板种子推导装载态、阶段、待处理操作与远端元信息。
pub(crate) fn panel_seed_state(
    seed: open::PanelSeed,
) -> (
    bool,
    panel::EditorPhase,
    Option<transom_core::TransferId>,
    Option<transom_core::RemoteTextMeta>,
    bool,
) {
    match seed {
        open::PanelSeed::Reading { op_id, .. } => {
            (false, panel::EditorPhase::Reading, Some(op_id), None, false)
        }
        open::PanelSeed::NewFile => (
            true,
            panel::EditorPhase::Idle,
            None,
            Some(transom_core::RemoteTextMeta {
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

pub(crate) use bridge::{
    RemoteEditorOpen, block_close_for_dirty_editors, cancel_editor_goto_line,
    commit_editor_goto_line, defer_editor_close_resume, note_editor_text_conflict,
    note_editor_text_failed, on_editor_text_written, on_sudo_probed, open_editor_goto_line,
    open_remote_editor_panel, open_terminal_code_location, prune_tab_overlays,
    render_editor_goto_line_body, sftp_edit_remote_file_on, spawn_new_editor_panel,
};
pub(crate) use close::{EditorCloseRequest, PendingEditorClose};
pub(crate) use diff::EditorDiffWriteSync;
pub(crate) use entity::{EditorDiffSource, EditorPanelEvent, EditorPanels};
pub(crate) use list::render_editor_list_popover;
pub(crate) use panel::{caret_of, editor_language_token};
