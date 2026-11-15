//! SFTP 删除确认弹窗视图实体。

use super::common::*;
use super::state::SftpPendingDelete;

/// SFTP 删除确认弹窗事件。
pub(crate) enum SftpConfirmModalEvent {
    Confirm,
    Cancel,
}

/// SFTP 删除确认弹窗实体视图。
pub(crate) struct SftpConfirmModalView {
    pub(crate) action: SftpPendingDelete,
    pub(crate) title: String,
    pub(crate) description: String,
}

impl EventEmitter<SftpConfirmModalEvent> for SftpConfirmModalView {}

impl Render for SftpConfirmModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl SftpConfirmModalView {
    pub(crate) fn new(action: SftpPendingDelete, title: String, description: String) -> Self {
        Self {
            action,
            title,
            description,
        }
    }

    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpConfirmModalEvent::Confirm);
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpConfirmModalEvent::Cancel);
    }

    fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let content = div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(self.description.clone());
        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-sftp-delete",
            rust_i18n::t!("common.cancel").to_string(),
            cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
        );
        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-sftp-delete",
            rust_i18n::t!("common.confirm_delete").to_string(),
            None,
            crate::widgets::DialogConfirmKind::Danger,
            cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx)),
        );
        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "sftp-confirm-modal",
            self.title.clone(),
            None::<String>,
            content,
            Some(footer),
        )
    }
}
