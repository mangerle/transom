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
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.set_transient_status(message, cx));
        }
    }

    /// 设置一条会自动过期的传输错误提示。
    ///
    /// 事件处理阶段外壳正持有面板的借用，因此错误必须等借用结束后经这里
    /// 回投实体，避免在面板已租借时再次更新同一实体。
    pub(crate) fn set_sftp_transient_error(
        &mut self,
        tab_id: TabId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.set_transient_queue_error(message, cx));
        }
    }
}
