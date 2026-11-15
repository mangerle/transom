//! SFTP 传输排除规则弹窗视图实体。

use super::common::*;
use gpui_kit::component::input::TextareaState;
use transom_core::validate_transfer_filter;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// SFTP 传输排除规则弹窗事件。
pub(crate) enum SftpFilterModalEvent {
    Save(String),
    Cancel,
}

/// SFTP 传输排除规则编辑弹窗实体视图。
pub(crate) struct SftpFilterModalView {
    pub(crate) textarea: Entity<TextareaState>,
    error: Option<String>,
}

impl EventEmitter<SftpFilterModalEvent> for SftpFilterModalView {}

impl Render for SftpFilterModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl SftpFilterModalView {
    pub(crate) fn new(textarea: Entity<TextareaState>) -> Self {
        Self {
            textarea,
            error: None,
        }
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.textarea.read(cx).value().to_string();
        if let Err(error) = validate_transfer_filter(&text) {
            self.error = Some(
                error
                    .detail
                    .unwrap_or_else(|| rust_i18n::t!("sftp.filter.invalid").to_string()),
            );
            self.textarea.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        cx.emit(SftpFilterModalEvent::Save(text));
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpFilterModalEvent::Cancel);
    }

    fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let content = v_flex()
            .gap_3()
            .child(Textarea::new(&self.textarea).h(px(220.0)).w_full())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.filter_rules_hint").to_string()),
            )
            .when_some(self.error.clone(), |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-sftp-filter",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "save-sftp-filter",
                rust_i18n::t!("sftp.modals.filter_save").to_string(),
                Some(AppIcon::Ui(UiIcon::CHECK)),
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)),
            ));

        crate::widgets::dialog_body(
            "sftp-filter-modal",
            rust_i18n::t!("sftp.filter_rules").to_string(),
            Some(rust_i18n::t!("sftp.modals.filter_scope_hint").to_string()),
            content,
            Some(footer),
        )
    }
}
