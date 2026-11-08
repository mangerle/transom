//! 连接确认弹窗的弹窗体渲染。
//!
//! 渲染归实体：按钮与开关统一经 `cx.listener` 回到 [`ConnectionPromptsView`]，
//! 实体不在渲染中反查外壳。三段弹窗体由 `auth_dialog.rs` 与
//! `clipboard_prompt.rs` 的原实现迁入，布局、文案与控件标识保持不变。
//!
//! Enter 与 Esc 不再由本文件单独接管：`dialog_host` 对每个弹窗体统一接输入框
//! 的 Enter/Esc 与容器动作，取消与提交两条落点指向同一实体方法。

use super::{AuthPromptState, ConnectionPromptsView};
use crate::dialog_host::ModalKind;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::PromptKind;

/// 剪贴板预览截断长度：足够让用户认出内容性质，又不会把弹窗撑爆。
const PREVIEW_CHARS: usize = 240;

impl ConnectionPromptsView {
    /// 按弹窗种类渲染对应主体；非本实体的种类返回空元素。
    pub(crate) fn render_body(
        &self,
        kind: ModalKind,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        match kind {
            ModalKind::AuthPrompt => self.render_auth(cx, p),
            ModalKind::HostKeyConfirm => self.render_host_key(cx, p),
            ModalKind::ClipboardPrompt => self.render_clipboard(cx, p),
            _ => div().into_any_element(),
        }
    }

    /// SSH 鉴权问答的弹窗体。
    fn render_auth(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let Some(prompt) = self.auth() else {
            return div().into_any_element();
        };
        let client_generated = prompt
            .prompts
            .iter()
            .any(|info| !matches!(info.kind, PromptKind::Text));
        let mut content = v_flex()
            .id(ElementId::Name("auth-prompt-body".into()))
            .max_h(px(360.0))
            .overflow_y_scroll()
            .gap_3();
        if !prompt.name.is_empty() {
            content = content.child(div().text_sm().child(prompt.name.clone()));
        }
        if !prompt.instructions.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(prompt.instructions.clone()),
            );
        }
        for (index, prompt_info) in prompt.prompts.iter().enumerate() {
            let input = prompt.inputs[index].clone();
            let label = match prompt_info.kind {
                PromptKind::Password => rust_i18n::t!("prompt.password_field").to_string(),
                PromptKind::Passphrase => rust_i18n::t!("prompt.passphrase_field").to_string(),
                PromptKind::Text => prompt_info.prompt.clone(),
            };
            let field = if prompt_info.echo {
                crate::widgets::form_field(label, input, p)
            } else {
                crate::widgets::secret_field(label, input, p)
            };
            content = content.child(field);
        }

        if prompt.secret_kind.is_some() {
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("prompt.save_credential").to_string()),
                    )
                    .child(
                        Switch::new("toggle-save-credential")
                            .small()
                            .checked(prompt.save)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_save(*checked, cx);
                            })),
                    ),
            );
        }

        let footer = render_auth_footer(prompt, cx, p);
        let hint = if client_generated {
            rust_i18n::t!("prompt.credential_hint").to_string()
        } else {
            rust_i18n::t!("prompt.auth_hint").to_string()
        };
        crate::widgets::dialog_body(
            "auth-prompt-modal",
            rust_i18n::t!("prompt.auth_title").to_string(),
            Some(hint),
            content,
            Some(footer),
        )
    }

    /// TOFU 首连主机指纹确认的弹窗体。
    ///
    /// 拒绝与 Esc、右上角关闭件走同一条路：必须把 `false` 发回 responder，
    /// 否则等待裁决的连接任务会永久挂住。
    fn render_host_key(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let Some(state) = self.host_key() else {
            return div().into_any_element();
        };
        let body = rust_i18n::t!(
            "prompt.host_key_body",
            host = state.host.as_str(),
            port = state.port
        )
        .to_string();
        let content = v_flex()
            .gap_3()
            .child(div().text_sm().child(body))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("{}: {}", state.key_type, state.host)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(p.mono_font_family.clone())
                            .child(state.fingerprint.clone()),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(p.danger)
                    .child(rust_i18n::t!("prompt.host_key_warning").to_string()),
            );
        let reject = crate::widgets::dialog_cancel_button(
            "reject-host-key",
            rust_i18n::t!("prompt.host_key_reject").to_string(),
            cx.listener(|this, _: &ClickEvent, _, cx| this.reject_host_key(cx)),
        );
        let accept = crate::widgets::dialog_confirm_button(
            "accept-host-key",
            rust_i18n::t!("prompt.host_key_trust").to_string(),
            None,
            crate::widgets::DialogConfirmKind::Primary,
            cx.listener(|this, _: &ClickEvent, _, cx| this.accept_host_key(cx)),
        );
        let footer =
            crate::widgets::dialog_actions([reject.into_any_element(), accept.into_any_element()]);
        crate::widgets::dialog_body(
            "host-key-modal",
            rust_i18n::t!("prompt.host_key_title").to_string(),
            Some(rust_i18n::t!("prompt.host_key_hint").to_string()),
            content,
            Some(footer),
        )
    }

    /// 剪贴板写入授权的弹窗体。
    ///
    /// 默认拒绝：Esc 与右上角关闭件都走 `deny_clipboard`，绝不因为"没点按钮"
    /// 就让等待中的 OSC 52 请求挂起。
    fn render_clipboard(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let Some(pending) = self.clipboard() else {
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
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.deny_clipboard(cx))),
            )
            .child(
                Button::new("allow-clipboard-write")
                    .primary()
                    .label(rust_i18n::t!("clipboard_consent.allow").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.allow_clipboard(cx))),
            );

        crate::widgets::dialog_body(
            "clipboard-consent-modal",
            rust_i18n::t!("clipboard_consent.title").to_string(),
            Some(rust_i18n::t!("clipboard_consent.subtitle").to_string()),
            content,
            Some(footer),
        )
    }
}

/// 认证提示页脚：错误、取消与提交。
fn render_auth_footer(
    prompt: &AuthPromptState,
    cx: &mut Context<ConnectionPromptsView>,
    p: &Palette,
) -> AnyElement {
    let mut footer = v_flex().gap_2().items_end();
    if let Some(error) = &prompt.error {
        footer = footer.child(div().text_sm().text_color(p.danger).child(error.clone()));
    }
    footer = footer.child(
        h_flex()
            .gap_2()
            .justify_end()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-auth",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_auth(cx)),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "submit-auth",
                rust_i18n::t!("prompt.submit").to_string(),
                None,
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, _, cx| this.submit_auth(cx)),
            )),
    );
    footer.into_any_element()
}
