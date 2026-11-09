//! 容器模块危险操作居中确认模态弹窗。

use crate::container::detail::short;
use crate::container::{ContainerConfirm, ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::*;

/// 渲染容器模块通用删除确认弹窗主体。
///
/// 设计原理（Why）：弹窗体只依赖工作区实体句柄、只读状态与 `Palette`；取消与
/// 确认回调直接 `update` 实体，不再绕外壳的 `dialog_host::on_event`。弹窗宿主只
/// 负责找到当前打开该弹窗的工作区并读取状态，渲染函数不反查外壳。
pub(crate) fn render_container_confirm_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let Some(confirm) = state.confirm.clone() else {
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

    let cancel = dialog_cancel_button(
        "cancel-container-confirm",
        rust_i18n::t!("common.cancel").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.cancel_container_confirm(cx));
            }
        },
    );

    let confirm_btn = dialog_confirm_button(
        "confirm-container-delete",
        rust_i18n::t!("common.confirm_delete").to_string(),
        Some(IconName::Trash),
        DialogConfirmKind::Danger,
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.apply_container_confirm(cx));
            }
        },
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm_btn.into_any_element()]);

    dialog_body(
        "container-confirm-modal",
        title,
        None::<String>,
        content,
        Some(footer),
    )
}
