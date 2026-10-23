//! 主题调色板、通用弹窗容器与错误文本本地化辅助工具。

use crate::app::AppShell;
use crate::keymap;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{AppError, ErrorCode};

/// 全局 UI 调色板快照，用于各子视图渲染统一配色。
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) background: Hsla,
    pub(crate) foreground: Hsla,
    pub(crate) border: Hsla,
    pub(crate) muted: Hsla,
    pub(crate) muted_foreground: Hsla,
    pub(crate) sidebar: Hsla,
    pub(crate) sidebar_foreground: Hsla,
    pub(crate) sidebar_border: Hsla,
    pub(crate) sidebar_accent: Hsla,
    pub(crate) sidebar_accent_foreground: Hsla,
    pub(crate) tab_bar: Hsla,
    pub(crate) tab_active: Hsla,
    pub(crate) tab_foreground: Hsla,
    pub(crate) tab_active_foreground: Hsla,
    pub(crate) title_bar: Hsla,
    pub(crate) title_bar_border: Hsla,
    pub(crate) status_bar: Hsla,
    pub(crate) danger: Hsla,
    pub(crate) danger_foreground: Hsla,
    pub(crate) success: Hsla,
    pub(crate) popover: Hsla,
    pub(crate) popover_foreground: Hsla,
}

impl Palette {
    pub(crate) fn from(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            background: theme.background,
            foreground: theme.foreground,
            border: theme.border,
            muted: theme.muted,
            muted_foreground: theme.muted_foreground,
            sidebar: theme.sidebar,
            sidebar_foreground: theme.sidebar_foreground,
            sidebar_border: theme.sidebar_border,
            sidebar_accent: theme.sidebar_accent,
            sidebar_accent_foreground: theme.sidebar_accent_foreground,
            tab_bar: theme.tab_bar,
            tab_active: theme.tab_active,
            tab_foreground: theme.tab_foreground,
            tab_active_foreground: theme.tab_active_foreground,
            title_bar: theme.title_bar,
            title_bar_border: theme.title_bar_border,
            status_bar: theme.status_bar,
            danger: theme.danger,
            danger_foreground: theme.danger_foreground,
            success: theme.success,
            popover: theme.popover,
            popover_foreground: theme.popover_foreground,
        }
    }
}

impl AppShell {
    pub(crate) fn dialog_frame(
        id: impl Into<ElementId>,
        title: impl Into<String>,
        description: Option<String>,
        content: impl IntoElement,
        footer: Option<impl IntoElement>,
        max_w: Pixels,
        p: &Palette,
    ) -> AnyElement {
        let title_str = title.into();
        v_flex()
            .id(id.into())
            .w(max_w)
            .max_w(px(760.0))
            .max_h(px(640.0))
            .gap_4()
            .p_6()
            .rounded_xl()
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .border_1()
            .border_color(p.border)
            .shadow_2xl()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child(title_str),
                    )
                    .when_some(description, |this, desc| {
                        this.child(div().text_xs().text_color(p.muted_foreground).child(desc))
                    }),
            )
            .child(content)
            .when_some(footer, |this, f| {
                this.child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .items_center()
                        .pt_2()
                        .child(f),
                )
            })
            .into_any_element()
    }

    pub(crate) fn overlay(content: impl IntoElement, _p: &Palette) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(Hsla { a: 0.55, ..black() })
            .key_context(keymap::OVERLAY_CONTEXT)
            // 遮罩必须阻断下层命中测试，否则点击输入框会同时触发终端/文件列表
            // 的鼠标处理并抢走焦点，表现为一输入就失焦。
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_down(MouseButton::Right, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(content)
            .into_any_element()
    }

    pub(crate) fn form_field(label: String, input: Entity<InputState>, p: &Palette) -> AnyElement {
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(p.muted_foreground).child(label))
            .child(Input::new(&input).w_full())
            .into_any_element()
    }
}

pub(crate) fn password_input(
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(rust_i18n::t!("form.password").to_string())
    })
}

pub(crate) fn sftp_text_input(
    window: &mut Window,
    cx: &mut Context<AppShell>,
    value: &str,
    placeholder: &str,
) -> Entity<InputState> {
    let value = value.to_string();
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(placeholder)
    })
}

pub(crate) fn localized_error(error: &AppError) -> String {
    let label = match error.code {
        ErrorCode::Configuration => rust_i18n::t!("error.configuration"),
        ErrorCode::ConnectionFailed => rust_i18n::t!("error.connection_failed"),
        ErrorCode::Dns => rust_i18n::t!("error.dns"),
        ErrorCode::Timeout => rust_i18n::t!("error.timeout"),
        ErrorCode::AuthFailed => rust_i18n::t!("error.auth_failed"),
        ErrorCode::KeyboardInteractiveFailed => {
            rust_i18n::t!("error.keyboard_interactive_failed")
        }
        ErrorCode::HostKeyUnknown => rust_i18n::t!("error.host_key_unknown"),
        ErrorCode::HostKeyMismatch => rust_i18n::t!("error.host_key_mismatch"),
        ErrorCode::ChannelFailed => rust_i18n::t!("error.channel_failed"),
        ErrorCode::SecretStoreUnavailable => rust_i18n::t!("error.secret_store_unavailable"),
        ErrorCode::Cancelled => rust_i18n::t!("error.cancelled"),
        ErrorCode::Protocol => rust_i18n::t!("error.protocol"),
        ErrorCode::Io => rust_i18n::t!("error.io"),
        ErrorCode::SftpFailed => rust_i18n::t!("error.sftp_failed"),
        ErrorCode::ForwardFailed => rust_i18n::t!("error.forward_failed"),
        _ => rust_i18n::t!("app.error"),
    }
    .to_string();
    match &error.detail {
        Some(detail) if !detail.is_empty() => format!("{label}: {detail}"),
        _ => label,
    }
}
