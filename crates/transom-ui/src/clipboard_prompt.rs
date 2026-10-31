//! 远端剪贴板写入（OSC 52）的按会话授权弹窗。
//!
//! 设计原理（Why）：
//! - 实现初衷：终端默认允许远端通过 OSC 52 静默覆盖本地剪贴板，等于任何
//!   在机器上跑的程序（含被入侵的远端）都能篡改用户粘贴的内容；默认必须拒绝。
//! - 核心优势：按"会话（连接）"授权而不是全局开关——同一台机器上，正在看
//!   的可信 TUI 不需要为一个可疑脚本背书；授权只活在当前终端视图里，断开重连
//!   自动失效，不会变成永久放行。
//! - 代价与局限：拒绝只作用于当前一次请求，程序每次复制都会再问一次；这是
//!   刻意选择的默认——剪贴板是跨信任边界的通道，宁可多问。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_core::TabId;

/// 预览截断长度：足够让用户认出内容性质，又不会把弹窗撑爆。
const PREVIEW_CHARS: usize = 240;

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
        if let Some(pending) = self.modals.clipboard_request.as_ref() {
            log::debug!(
                "已有待裁决的远端剪贴板请求，本次请求被忽略，窗格: {tab_id}，待裁决窗格: {}",
                pending.tab_id
            );
            return;
        }
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let session_name = tab.session.name.clone();
        self.modals.clipboard_request = Some(ClipboardWritePrompt {
            tab_id,
            session_name,
            text,
        });
        cx.notify();
    }

    /// 允许本会话写入：授予会话级权限，并把本次待写内容落盘。
    ///
    /// 授权位落在终端视图上（连接级），本次内容在这里直接写入：视图的授权方法
    /// 只改变后续请求的走向，不重复写剪贴板。
    pub(crate) fn confirm_clipboard_write(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.modals.clipboard_request.take() else {
            return;
        };
        let terminal = self
            .workspace
            .tab_mut(pending.tab_id)
            .and_then(|tab| tab.session_pane_mut())
            .and_then(|pane| pane.terminal.clone());
        if let Some(terminal) = terminal {
            terminal.update(cx, |view, _| view.allow_remote_clipboard_write());
        }
        cx.write_to_clipboard(ClipboardItem::new_string(pending.text));
        self.status = Some(rust_i18n::t!("clipboard_consent.allowed").to_string());
        cx.notify();
    }

    /// 拒绝本次写入；不改变会话授权状态，下次请求仍会询问。
    pub(crate) fn deny_clipboard_write(&mut self, cx: &mut Context<Self>) {
        self.modals.clipboard_request = None;
        self.status = Some(rust_i18n::t!("clipboard_consent.denied").to_string());
        cx.notify();
    }

    /// 剪贴板写入授权的弹窗体。
    ///
    /// 默认拒绝：Esc 与右上角关闭件都走 `deny_clipboard_write`，把拒绝回给
    /// 等待中的 OSC 52 请求，绝不因为"没点按钮"就让任务挂起。
    pub(crate) fn render_clipboard_prompt_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(pending) = self.modals.clipboard_request.as_ref() else {
            return div().into_any_element();
        };
        let session_name = pending.session_name.clone();
        let total = pending.text.chars().count();
        // 预览只保留开头一段；换行替换成可见符号，其余控制字符折成空格，
        // 避免 ESC 等不可见字节把布局或文本渲染搅乱。
        let preview: String = pending
            .text
            .chars()
            .take(PREVIEW_CHARS)
            .map(|ch| match ch {
                '\n' => '⏎',
                ch if ch.is_control() => ' ',
                ch => ch,
            })
            .collect();
        let truncated = total > PREVIEW_CHARS;

        let content = v_flex()
            .gap_3()
            .child(
                div().text_xs().text_color(p.muted_foreground).child(
                    rust_i18n::t!("clipboard_consent.hint", session = session_name.as_str())
                        .to_string(),
                ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!("clipboard_consent.preview", count = total).to_string(),
                    ))
                    .child(
                        div()
                            .id(ElementId::Name("clipboard-consent-preview".into()))
                            .w_full()
                            .max_h(px(120.0))
                            .overflow_y_scroll()
                            .p_2()
                            .rounded(p.radius.sm)
                            .bg(p.muted)
                            .text_xs()
                            .child(if truncated {
                                format!("{preview}…")
                            } else {
                                preview
                            }),
                    ),
            );

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("deny-clipboard-write")
                    .ghost()
                    .label(rust_i18n::t!("clipboard_consent.deny").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.deny_clipboard_write(cx),
                    )),
            )
            .child(
                Button::new("allow-clipboard-write")
                    .primary()
                    .label(rust_i18n::t!("clipboard_consent.allow").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.confirm_clipboard_write(cx),
                    )),
            );

        Self::dialog_body(
            "clipboard-consent-modal",
            rust_i18n::t!("clipboard_consent.title").to_string(),
            Some(rust_i18n::t!("clipboard_consent.subtitle").to_string()),
            content,
            Some(footer),
        )
    }
}
