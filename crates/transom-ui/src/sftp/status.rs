//! SFTP 面板瞬时状态消息的过期管理。

use super::SftpPanelState;
use crate::app::AppShell;
use gpui_kit::*;
use std::time::Duration;
use transom_core::TabId;

/// 成功提示在状态栏的停留时间。
///
/// 设计原理（Why）：传输完成、校验通过属于瞬时反馈，用户确认后不应继续
/// 占据状态栏；5 秒足够扫一眼，又不至于让状态栏长期停留在旧结果。
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

impl AppShell {
    /// 设置一条会在一段时间后自动消失的面板提示。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：面板的 `status` 是单一字符串，多个写入点若各自维护定时器
    ///   容易互相覆盖；集中到本方法统一挂载过期任务；
    /// - 核心优势：新消息会替换并取消旧的过期任务；任务到点后还会比对消息
    ///   内容，若期间已被别的操作覆盖则不误清；
    /// - 代价与局限：只有经过本方法的提示会自动过期；错误消息走 `error`
    ///   字段，保持到下一次操作，避免用户还没看到就被清掉。
    pub(crate) fn set_sftp_transient_status(
        panel: &mut SftpPanelState,
        tab_id: TabId,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let expiry_message = message.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_MESSAGE_TTL).await;
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = this
                    .workspace
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| tab.sftp_mut())
                else {
                    return;
                };
                if panel.status.as_deref() == Some(expiry_message.as_str()) {
                    panel.status = None;
                    cx.notify();
                }
            });
        });
        panel.status = Some(message);
        panel.status_task = Some(task);
    }
}
