//! SFTP 面板瞬时状态消息的过期管理（外壳入口）。
//!
//! 消息与过期任务归实体，外壳只保留按标签转发的薄委托，供 SFTP 事件处理
//! 在实体非租借期调用。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 设置一条会在一段时间后自动消失的面板提示。
    pub(crate) fn set_sftp_transient_status(
        &mut self,
        tab_id: TabId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.set_transient_status(message, cx));
        }
    }
}
