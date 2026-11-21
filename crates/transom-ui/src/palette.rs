//! 调色板重导出、弹窗遮罩与错误本地化。

pub(crate) use transom_ui_kit::palette::*;

use crate::keymap;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{AppError, ErrorCode};

/// 自绘遮罩浮层：仅命令面板还在用。
///
/// 其余模态已迁入组件库的弹窗栈（见 `dialog_host`），那里遮罩、焦点捕获、
/// 层序与关闭件都由容器负责。这条保留是因为命令面板不是模态弹窗语义：
/// 它要"Esc 先清查询再关闭"，且输入框焦点必须留在 `AppShell` 子树内，
/// 才能继续靠 `Workspace && !Overlay` 挡住工作区快捷键。
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

pub(crate) fn sftp_text_input<T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
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
        ErrorCode::FtpFailed => rust_i18n::t!("error.ftp_failed"),
        ErrorCode::NotSupported => rust_i18n::t!("error.not_supported"),
        ErrorCode::PermissionDenied => rust_i18n::t!("error.permission_denied"),
        ErrorCode::SftpVerifyFailed => rust_i18n::t!("error.sftp_verify_failed"),
        ErrorCode::ForwardFailed => rust_i18n::t!("error.forward_failed"),
        ErrorCode::CloudSyncFailed => rust_i18n::t!("error.cloud_sync_failed"),
        ErrorCode::CloudSyncConflict => rust_i18n::t!("error.cloud_sync_conflict"),
        ErrorCode::CloudSyncMissing => rust_i18n::t!("error.cloud_sync_missing"),
        _ => rust_i18n::t!("app.error"),
    }
    .to_string();
    // 底层 `AppError.detail` 是 core/net 产出的中文诊断文本；只有中文界面
    // 才拼接，其他语言只展示按错误码本地化的标题，避免中英混排的"半句英文"。
    // 需要完整诊断时以日志为准，界面不再充当原始错误的透传管道。
    match &error.detail {
        Some(detail) if !detail.is_empty() && locale_displays_detail(&rust_i18n::locale()) => {
            format!("{label}: {detail}")
        }
        _ => label,
    }
}

/// 指定语言是否应展示底层中文诊断详情。
///
/// 纯函数便于测试：`zh`、`zh-CN`、`zh-TW` 等中文变体展示详情，其余语言
/// 只显示按错误码本地化的标题。
fn locale_displays_detail(locale: &str) -> bool {
    locale.starts_with("zh")
}

#[cfg(test)]
mod tests {
    use super::locale_displays_detail;

    #[test]
    fn chinese_locales_show_raw_detail() {
        assert!(locale_displays_detail("zh"));
        assert!(locale_displays_detail("zh-CN"));
        assert!(locale_displays_detail("zh-TW"));
    }

    #[test]
    fn other_locales_hide_raw_detail() {
        assert!(!locale_displays_detail("en"));
        assert!(!locale_displays_detail("en-US"));
        assert!(!locale_displays_detail("ja"));
    }
}
