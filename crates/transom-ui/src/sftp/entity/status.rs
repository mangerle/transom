//! 实体侧的瞬时状态消息与过期任务。
//!
//! 状态写入与过期判断都只看面板状态，因此归实体；过期任务在实体上下文里
//! 挂载，实体销毁时任务随之取消，不会回写到已关闭的窗格。

use super::*;
use std::time::Duration;

/// 成功提示在状态栏的停留时间。
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

impl SftpPanelView {
    /// 设置一条会在一段时间后自动消失的面板提示。
    ///
    /// 新消息会替换并取消旧的过期任务；到点后还会比对消息内容，期间已被
    /// 别的操作覆盖则不误清。
    pub(crate) fn set_transient_status(&mut self, message: String, cx: &mut Context<Self>) {
        let expiry_message = message.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_MESSAGE_TTL).await;
            let _ = this.update(cx, |view, _| {
                if view.queue.status.as_deref() == Some(expiry_message.as_str()) {
                    view.clear_status();
                }
            });
        });
        self.state.set_status(message);
        self.state.queue.status_task = Some(task);
    }
}
