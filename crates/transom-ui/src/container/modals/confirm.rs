//! 容器模块危险操作居中确认模态弹窗。

use crate::app::AppShell;
use crate::container::detail::short;
use crate::container::types::ContainerConfirm;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::*;

impl AppShell {
    /// 渲染容器模块通用删除确认弹窗主体。
    pub(crate) fn render_container_confirm_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some((tab_id, workspace)) = self.find_container_modal_tab(|ws| ws.confirm.is_some())
        else {
            return div().into_any_element();
        };
        let Some(confirm) = workspace.confirm.clone() else {
            return div().into_any_element();
        };

        let (title, message) = match &confirm {
            ContainerConfirm::Container(name) => (
                rust_i18n::t!("container.confirm.title_container").to_string(),
                rust_i18n::t!("container.confirm.remove_container", name = name).to_string(),
            ),
            ContainerConfirm::Image(image) => (
                rust_i18n::t!("container.confirm.title_image").to_string(),
                rust_i18n::t!("container.confirm.remove_image", name = short(image)).to_string(),
            ),
            ContainerConfirm::Volume(name) => (
                rust_i18n::t!("container.confirm.title_volume").to_string(),
                rust_i18n::t!("container.confirm.remove_volume", name = name).to_string(),
            ),
            ContainerConfirm::Network(name) => (
                rust_i18n::t!("container.confirm.title_network").to_string(),
                rust_i18n::t!("container.confirm.remove_network", name = name).to_string(),
            ),
            ContainerConfirm::Compose(name) => (
                rust_i18n::t!("container.confirm.title_compose").to_string(),
                rust_i18n::t!("container.confirm.compose_down", name = name).to_string(),
            ),
            ContainerConfirm::FilePath { path, .. } => (
                rust_i18n::t!("container.confirm.title_file").to_string(),
                rust_i18n::t!("container.confirm.remove_path", path = path).to_string(),
            ),
            ContainerConfirm::UploadOverwrite { path, .. } => (
                rust_i18n::t!("container.confirm.title_upload_overwrite").to_string(),
                rust_i18n::t!("container.confirm.upload_overwrite", path = path).to_string(),
            ),
        };

        let content = v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.foreground)
                    .child(message),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.danger)
                    .child(rust_i18n::t!("container.confirm.irreversible").to_string()),
            );

        let cancel = crate::widgets::dialog_cancel_button(
            "cancel-container-confirm",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, move |this, _: &ClickEvent, _, cx| {
                this.cancel_container_confirm(tab_id, cx);
            }),
        );

        let confirm_btn = crate::widgets::dialog_confirm_button(
            "confirm-container-delete",
            rust_i18n::t!("common.confirm_delete").to_string(),
            Some(IconName::Trash),
            crate::widgets::DialogConfirmKind::Danger,
            crate::dialog_host::on_event(app, {
                let confirm = confirm.clone();
                move |this, _: &ClickEvent, _, cx| {
                    this.apply_container_confirm(tab_id, confirm.clone(), cx);
                }
            }),
        );

        let footer = crate::widgets::dialog_actions([
            cancel.into_any_element(),
            confirm_btn.into_any_element(),
        ]);

        crate::widgets::dialog_body(
            "container-confirm-modal",
            title,
            None::<String>,
            content,
            Some(footer),
        )
    }
}
