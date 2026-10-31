//! SFTP 跨列剪贴板的外壳入口。
//!
//! 剪贴板状态、选中集与传输编排都归实体；外壳只按标签转发，供面板容器的
//! 键盘监听与右键菜单回调调用。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 触发剪贴板复制（Ctrl+C 或右键复制）。
    pub(crate) fn sftp_copy_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.copy_selection(cx));
        }
    }

    /// 触发剪贴板粘贴（Ctrl+V 或右键粘贴）。
    pub(crate) fn sftp_paste_clipboard(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.paste_clipboard(cx));
        }
    }
}
