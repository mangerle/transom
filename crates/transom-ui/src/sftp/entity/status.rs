//! 实体侧的瞬时状态消息与过期任务。
//!
//! 状态写入与过期判断都只看面板状态，因此归实体；过期任务在实体上下文里
//! 挂载，实体销毁时任务随之取消，不会回写到已关闭的窗格。

use super::*;
use std::time::Duration;

/// 成功提示在状态栏的停留时间。
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

/// 传输错误提示在状态栏的停留时间。
///
/// 失败原因的持久记录在队列任务行上，底栏只是提醒；不设过期就会一直占位，
/// 用户既清不掉也看不到后续状态。取值比成功提示长，留出读完一句话的时间。
const ERROR_MESSAGE_TTL: Duration = Duration::from_secs(12);

impl SftpPanelView {
    /// 设置一条会在一段时间后自动消失的面板提示。
    ///
    /// 新消息会替换并取消旧的过期任务；到点后还会比对消息内容，期间已被
    /// 别的操作覆盖则不误清。清空状态后必须主动重绘，否则旧文案会留在屏幕上
    /// 直到下一次由别的事件触发的重绘。
    pub(crate) fn set_transient_status(&mut self, message: String, cx: &mut Context<Self>) {
        let expiry_message = message.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_MESSAGE_TTL).await;
            let _ = this.update(cx, |view, cx| {
                if view.state.queue.status.as_deref() == Some(expiry_message.as_str()) {
                    view.state.clear_status();
                    cx.notify();
                }
            });
        });
        self.state.set_status(message);
        self.state.queue.status_task = Some(task);
    }

    /// 设置一条会自动过期的传输队列错误提示。
    ///
    /// 设计原理（Why）：传输失败的持久记录是任务行上的失败原因，底栏只承担
    /// “刚刚出错了”的提醒职责。提醒若永久驻留，会把底栏变成一条无法清除的
    /// 历史文本，用户看不到后续操作状态；到期或手动关闭后清除，任务行仍保留
    /// 完整原因。
    ///
    /// 代价与局限：用户离开窗口超过有效期会错过底栏提醒，但仍可从任务行看到
    /// 失败原因并重试。
    pub(crate) fn set_transient_queue_error(&mut self, message: String, cx: &mut Context<Self>) {
        let expiry_message = message.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ERROR_MESSAGE_TTL).await;
            let _ = this.update(cx, |view, cx| {
                if view.state.queue.error.as_deref() == Some(expiry_message.as_str()) {
                    view.state.clear_queue_error();
                    cx.notify();
                }
            });
        });
        self.state.set_queue_error(message);
        self.state.queue.error_task = Some(task);
    }

    /// 手动关闭底栏错误提示：失败记录仍保留在任务行上。
    pub(crate) fn dismiss_sftp_errors(&mut self, cx: &mut Context<Self>) {
        self.state.clear_queue_error();
        self.state.clear_local_error();
        cx.notify();
    }
}
