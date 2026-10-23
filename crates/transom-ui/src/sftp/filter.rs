//! SFTP 目录传输排除规则的编辑弹窗操作。

use super::SftpFilterModalState;
use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::component::input::TextareaState;
use gpui_kit::*;
use transom_core::validate_transfer_filter;

impl AppShell {
    pub(crate) fn sftp_open_filter_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.settings.sftp.transfer_filter.clone();
        let textarea = cx.new(|cx| TextareaState::new(window, cx).default_value(text));
        textarea.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_filter_modal = Some(SftpFilterModalState {
            textarea,
            error: None,
        });
        cx.notify();
    }

    /// 校验并保存排除规则；非法规则保留弹窗并就地提示，不中断已有传输。
    pub(crate) fn save_sftp_filter_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut modal) = self.sftp_filter_modal.take() else {
            return;
        };
        let text = modal.textarea.read(cx).value().to_string();
        if let Err(error) = validate_transfer_filter(&text) {
            modal.error = Some(
                error
                    .detail
                    .unwrap_or_else(|| "传输过滤规则无效".to_string()),
            );
            let textarea = modal.textarea.clone();
            self.sftp_filter_modal = Some(modal);
            textarea.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        self.settings.sftp.transfer_filter = text;
        match self.services.settings_store.save(&self.settings) {
            Ok(()) => {
                self.status = Some(rust_i18n::t!("sftp.filter_rules_saved").to_string());
            }
            Err(error) => {
                self.error = Some(localized_error(&error));
            }
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }
}
