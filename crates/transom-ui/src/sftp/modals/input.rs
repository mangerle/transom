//! SFTP 输入弹窗视图实体。

use super::common::*;
use crate::palette::sftp_text_input;
use crate::sftp::SftpInputTarget;
use gpui_kit::component::input::InputState;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 输入弹窗的装配参数。
#[derive(Clone)]
pub(crate) struct SftpInputModalParams {
    pub(crate) tab_id: TabId,
    /// `None` 表示主远端；`Some` 表示并列远端列。本地目标也使用 `None`。
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) placeholder: String,
    pub(crate) value: String,
    pub(crate) target: SftpInputTarget,
}

/// SFTP 输入弹窗事件。
pub(crate) enum SftpInputModalEvent {
    Submit {
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        target: SftpInputTarget,
        value: String,
    },
    Cancel,
}

/// SFTP 输入弹窗实体视图：新建、重命名、软链接、归档与提权口令共用。
pub(crate) struct SftpInputModalView {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) prompt: String,
    pub(crate) input: Entity<InputState>,
    pub(crate) target: SftpInputTarget,
    pub(crate) error: Option<String>,
}

impl EventEmitter<SftpInputModalEvent> for SftpInputModalView {}

impl Render for SftpInputModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl SftpInputModalView {
    pub(crate) fn new(
        params: SftpInputModalParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = sftp_text_input(window, cx, &params.value, &params.placeholder);
        input.update(cx, |state, cx| state.focus(window, cx));
        Self {
            tab_id: params.tab_id,
            connection_id: params.connection_id,
            title: params.title,
            prompt: params.prompt,
            input,
            target: params.target,
            error: None,
        }
    }

    pub(crate) fn set_error(
        &mut self,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = Some(error);
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().trim().to_string();
        if value.is_empty() {
            self.set_error(
                rust_i18n::t!("sftp.file_ops.name_empty").to_string(),
                window,
                cx,
            );
            return;
        }
        if let Some(message) = crate::sftp::file_ops::validate::name_validation_error(&self.target, &value)
        {
            self.set_error(message, window, cx);
            return;
        }
        cx.emit(SftpInputModalEvent::Submit {
            tab_id: self.tab_id,
            connection_id: self.connection_id,
            target: self.target.clone(),
            value,
        });
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpInputModalEvent::Cancel);
    }

    fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let content = v_flex()
            .gap_3()
            .child(Input::new(&self.input).w_full())
            .when_some(self.error.clone(), |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-sftp-input",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "confirm-sftp-input",
                rust_i18n::t!("common.confirm").to_string(),
                Some(AppIcon::Ui(UiIcon::CHECK)),
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)),
            ));

        crate::widgets::dialog_body(
            "sftp-input-modal",
            self.title.clone(),
            Some(self.prompt.clone()),
            content,
            Some(footer),
        )
    }
}
