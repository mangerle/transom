//! 容器资源限制更新居中模态弹窗。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    /// 渲染容器资源限制更新模态弹窗主体。
    pub(crate) fn render_container_update_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) =
            self.find_container_modal_tab(|ws| ws.update_container.is_some())
        else {
            return div().into_any_element();
        };
        let Some((_, mem_input, cpu_input)) = workspace.update_container.clone() else {
            return div().into_any_element();
        };

        let content = v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.muted_foreground)
                            .child(
                                rust_i18n::t!("container.update_resources.memory_label")
                                    .to_string(),
                            ),
                    )
                    .child(Input::new(&mem_input).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.muted_foreground)
                            .child(
                                rust_i18n::t!("container.update_resources.cpu_label").to_string(),
                            ),
                    )
                    .child(Input::new(&cpu_input).w_full()),
            );

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-update",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_update(tab_id, cx);
            }),
        );

        let confirm = crate::widgets::dialog_confirm_button(
            "confirm-container-update",
            rust_i18n::t!("common.confirm").to_string(),
            Some(IconName::Check),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.submit_container_update(tab_id, cx);
            }),
        );

        let footer =
            crate::widgets::dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

        crate::widgets::dialog_body(
            "container-update-modal",
            rust_i18n::t!("container.update_resources.modal_title").to_string(),
            Some(rust_i18n::t!("container.update_resources.prompt").to_string()),
            content,
            Some(footer),
        )
    }
}
