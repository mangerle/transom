//! 远端剪贴板写入（OSC 52）的按会话授权入口与待裁决请求。
//!
//! 设计原理（Why）：
//! - 实现初衷：终端默认允许远端通过 OSC 52 静默覆盖本地剪贴板，等于任何
//!   在机器上跑的程序（含被入侵的远端）都能篡改用户粘贴的内容；默认必须拒绝。
//! - 核心优势：按"会话（连接）"授权而不是全局开关——同一台机器上，正在看
//!   的可信 TUI 不需要为一个可疑脚本背书；授权只活在当前终端视图里，断开重连
//!   自动失效，不会变成永久放行。
//! - 代价与局限：拒绝只作用于当前一次请求，程序每次复制都会再问一次；这是
//!   刻意选择的默认——剪贴板是跨信任边界的通道，宁可多问。
//!
//! 待裁决请求存在 `ConnectionPromptsView` 实体里，弹窗体与允许/拒绝动作见
//! `connection/prompts_render.rs` 与 `connection/prompts_view.rs`；本文件只保留
//! 请求数据与"登记一次待裁决请求"的外壳入口。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::TabId;

/// 一次等待用户裁决的 OSC 52 写请求。
pub(crate) struct ClipboardWritePrompt {
    /// 发起请求的窗格；授权与落盘都作用于它。
    pub(crate) tab_id: TabId,
    /// 会话名，用于让用户确认来源。
    pub(crate) session_name: String,
    pub(crate) text: String,
}

impl AppShell {
    /// 登记一次远端剪贴板写入请求。
    ///
    /// 已有待裁决请求时静默丢弃后来的（无论来自哪个窗格）：弹窗只有一份，
    /// 覆盖会让前一个请求凭空消失；而程序往往在每次复制时连发，排队弹窗
    /// 也只会把界面淹没，用户的一次决定足以覆盖这段连续请求。
    pub(crate) fn request_clipboard_permission(
        &mut self,
        tab_id: TabId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let prompts = self.state.modals.connection.prompts.clone();
        if let Some(pending) = prompts.read(cx).clipboard() {
            log::debug!(
                "已有待裁决的远端剪贴板请求，本次请求被忽略，窗格: {tab_id}，待裁决窗格: {}",
                pending.tab_id
            );
            return;
        }
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return;
        };
        let session_name = tab.session.name.clone();
        prompts.update(cx, |view, cx| {
            view.open_clipboard(
                ClipboardWritePrompt {
                    tab_id,
                    session_name,
                    text,
                },
                cx,
            );
        });
        cx.notify();
    }
}
