//! SFTP 编辑面板的"跳转到行"状态与光标定位辅助。

use gpui_kit::component::input::{EditorState, InputState, Position, RopeExt};
use gpui_kit::*;

/// 编辑器"跳转到行"弹窗的状态。
pub(crate) struct EditorGotoLineModalState {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
    pub(crate) _enter_sub: Subscription,
}

/// 把编辑器光标移到 1 基的行列位置，展开被折叠的行并收敛越界值。
///
/// 收敛口径集中在这里：空文件至少第 1 行、超界跳到最后一行，列缺省为行首。
/// "跳转到行"模态与终端代码链接共用同一套定位行为，避免两处各算一套边界。
pub(crate) fn apply_editor_goto(
    editor: &Entity<EditorState>,
    line: u32,
    column: Option<u32>,
    window: &mut Window,
    cx: &mut App,
) {
    let total = editor.read(cx).text().lines_len().max(1) as u32;
    let line = line.max(1).min(total);
    let column = column.unwrap_or(1).max(1) - 1;
    let position = Position::new(line - 1, column);
    editor.update(cx, |state, cx| {
        state.unfold_at(position, cx);
        state.set_cursor_position(position, window, cx);
    });
}
