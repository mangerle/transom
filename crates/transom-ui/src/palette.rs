//! 主题调色板、通用弹窗容器与错误文本本地化辅助工具。

use crate::app::AppShell;
use crate::keymap;
use crate::theme::tokens::size;
use gpui_kit::base::{RadiusTokens, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{AppError, ErrorCode};

/// 全局 UI 调色板快照，用于各子视图渲染统一配色。
///
/// 为什么是值快照而不是直接读 `cx.theme()`：渲染函数签名只依赖 `&Palette`，
/// 不与 `cx` 借用纠缠；同时每帧从全局主题重建，主题热切换后下一帧自动生效。
/// 含 `SharedString` 字段因此只能 Clone，各渲染闭包持有 `Palette` 值时用
/// `p.clone()` 传递。
#[derive(Clone)]
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
    /// 列表当前项的强调边框色，用于把"正在使用的会话"与普通悬浮态区分开。
    pub(crate) list_active_border: Hsla,
    pub(crate) tab_bar: Hsla,
    pub(crate) tab_active: Hsla,
    pub(crate) tab_foreground: Hsla,
    pub(crate) tab_active_foreground: Hsla,
    pub(crate) title_bar: Hsla,
    pub(crate) title_bar_border: Hsla,
    pub(crate) status_bar: Hsla,
    /// 主题强调色（设置页主题卡预览色点等场景）。
    pub(crate) primary: Hsla,
    /// 警示语义色（设置页明暗反差提示等场景）。
    pub(crate) warning: Hsla,
    pub(crate) danger: Hsla,
    pub(crate) danger_foreground: Hsla,
    pub(crate) danger_hover: Hsla,
    pub(crate) danger_active: Hsla,
    pub(crate) success: Hsla,
    /// 模态遮罩底色：自带透明度，随主题明暗变化。
    pub(crate) overlay: Hsla,
    pub(crate) popover: Hsla,
    pub(crate) popover_foreground: Hsla,
    /// 界面等宽字体族（SFTP 预览、指纹展示等逐字符对齐场景）。
    pub(crate) mono_font_family: SharedString,
    /// 界面等宽字号，随密度档位缩放。
    pub(crate) mono_font_size: Pixels,
    /// 圆角档位（由主题的 radius/radius_lg 派生），容器级圆角统一从这里取值，
    /// 主题即可控制全局圆角；装饰性圆点仍用 rounded_full。
    pub(crate) radius: RadiusTokens,
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
            list_active_border: theme.list_active_border,
            tab_bar: theme.tab_bar,
            tab_active: theme.tab_active,
            tab_foreground: theme.tab_foreground,
            tab_active_foreground: theme.tab_active_foreground,
            title_bar: theme.title_bar,
            title_bar_border: theme.title_bar_border,
            status_bar: theme.status_bar,
            primary: theme.primary,
            warning: theme.warning,
            danger: theme.danger,
            danger_foreground: theme.danger_foreground,
            danger_hover: theme.danger_hover,
            danger_active: theme.danger_active,
            success: theme.success,
            overlay: theme.overlay,
            popover: theme.popover,
            popover_foreground: theme.popover_foreground,
            mono_font_family: theme.mono_font_family.clone(),
            mono_font_size: theme.mono_font_size,
            radius: theme.radius_tokens(),
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
        max_w: impl Into<Length>,
        p: &Palette,
    ) -> AnyElement {
        let title_str = title.into();
        // 先固化为 Copy 的 Length，避免 impl Trait 参数在链式调用里要求 Clone。
        let max_w = max_w.into();
        v_flex()
            .id(id.into())
            .w(max_w)
            .max_w(rems(size::DIALOG_MAX_WIDTH))
            .max_h(rems(size::DIALOG_MAX_HEIGHT))
            .gap_4()
            .p_6()
            .rounded(p.radius.xl)
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

    pub(crate) fn overlay(content: impl IntoElement, p: &Palette) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.overlay)
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
        Self::field_with_reveal(label, input, false, p)
    }

    /// 机密字段（登录密码、私钥口令）：在普通字段之上启用输入框自带的明文切换按钮。
    ///
    /// 复用组件库的 `mask_toggle` 而不是自己画一只眼睛：图标朝向、无障碍标签与掩码
    /// 状态都由输入框自身维护，切换只影响显示，取值仍是真实文本。
    pub(crate) fn secret_field(
        label: String,
        input: Entity<InputState>,
        p: &Palette,
    ) -> AnyElement {
        Self::field_with_reveal(label, input, true, p)
    }

    fn field_with_reveal(
        label: String,
        input: Entity<InputState>,
        reveal: bool,
        p: &Palette,
    ) -> AnyElement {
        let field = Input::new(&input).w_full();
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(p.muted_foreground).child(label))
            .child(if reveal { field.mask_toggle() } else { field })
            .into_any_element()
    }
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
        ErrorCode::BackupEncrypted => rust_i18n::t!("error.backup_encrypted"),
        ErrorCode::BackupNotEncrypted => rust_i18n::t!("error.backup_not_encrypted"),
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
