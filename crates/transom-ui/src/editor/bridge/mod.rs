//! 远端文本编辑器与外壳之间的唯一桥接层。
//!
//! 设计原理（Why）：
//! - 除本子模块外，`editor/` 下全部业务、渲染、状态与编解码文件彻底脱离
//!   `AppShell`；
//! - 所有跨面板协调（SFTP 命令发送、提权口令弹窗、标签关闭拦截、跳行弹窗、
//!   终端代码链接打开）全部收束在本模块，按职责分为 `ops`、`lifecycle`、`open` 三个子文件。

mod lifecycle;
mod open;
mod ops;

pub(crate) use lifecycle::{
    block_close_for_dirty_editors, cancel_editor_goto_line, commit_editor_goto_line,
    defer_editor_close_resume, open_editor_goto_line, prune_tab_overlays,
    render_editor_goto_line_body,
};
pub(crate) use open::{
    RemoteEditorOpen, open_remote_editor_panel, open_terminal_code_location,
    sftp_edit_remote_file_on, spawn_new_editor_panel,
};
pub(crate) use ops::{
    editor_reject, note_editor_text_conflict, note_editor_text_failed, on_editor_text_written,
    on_sudo_probed,
};
