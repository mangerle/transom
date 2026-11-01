//! SFTP 目录传输排除规则的编辑弹窗操作。

use super::SftpFilterModalState;
use super::entity::{SftpPanelView, read_shell_parts, with_shell_parts};
use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::component::input::TextareaState;
use gpui_kit::*;
use transom_core::validate_transfer_filter;

impl SftpPanelView {
    /// 打开传输排除规则编辑弹窗：文本取自外壳设置，装配在外壳完成。
    pub(crate) fn open_filter_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let Some(text) = read_shell_parts(&app, cx, |shell| {
            shell.state.settings.sftp.transfer_filter.clone()
        }) else {
            return;
        };
        with_shell_parts(&app, cx, |shell, shell_cx| {
            let textarea = shell_cx.new(|cx| TextareaState::new(window, cx).default_value(text));
            textarea.update(shell_cx, |state, cx| state.focus(window, cx));
            shell.state.sftp_modals.filter = Some(SftpFilterModalState {
                textarea,
                error: None,
            });
            shell_cx.notify();
        });
    }
}

impl AppShell {
    /// 校验并保存排除规则；非法规则保留弹窗并就地提示，不中断已有传输。
    pub(crate) fn save_sftp_filter_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut modal) = self.state.sftp_modals.filter.take() else {
            return;
        };
        let text = modal.textarea.read(cx).value().to_string();
        if let Err(error) = validate_transfer_filter(&text) {
            modal.error = Some(
                error
                    .detail
                    .unwrap_or_else(|| rust_i18n::t!("sftp.filter.invalid").to_string()),
            );
            let textarea = modal.textarea.clone();
            self.state.sftp_modals.filter = Some(modal);
            textarea.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        self.state.settings.sftp.transfer_filter = text;
        let settings_snapshot = self.state.settings.clone();
        match self.state.persist_settings(&settings_snapshot) {
            Ok(()) => {
                self.state.status = Some(rust_i18n::t!("sftp.filter_rules_saved").to_string());
            }
            Err(error) => {
                self.state.set_error(localized_error(&error));
            }
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }
}
