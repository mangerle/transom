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
mod navigate;
mod open;
mod open_support;
mod ops;
mod panel;
mod status;
mod view;

pub(crate) use bridge::{
    RemoteEditorOpen, block_close_for_dirty_editors, cancel_editor_goto_line,
    commit_editor_goto_line, defer_editor_close_resume, note_editor_text_conflict,
    note_editor_text_failed, on_editor_text_written, on_sudo_probed, open_editor_goto_line,
    open_remote_editor_panel, open_terminal_code_location, prune_tab_overlays,
    render_editor_goto_line_body, sftp_edit_remote_file_on, spawn_new_editor_panel,
    submit_sudo_editor_write,
};
pub(crate) use close::PendingEditorClose;
pub(crate) use diff::EditorDiffWriteSync;
pub(crate) use entity::{EditorDiffSource, EditorPanelEvent, EditorPanels};
pub(crate) use list::render_editor_list_popover;
pub(crate) use navigate::EditorGotoLineModalState;
pub(crate) use panel::{caret_of, editor_language_token};
