//! 容器重命名居中模态弹窗。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

/// 渲染容器重命名模态弹窗主体。
///
/// 设计原理（Why）：输入控件与打开状态都归工作区实体，渲染只读状态快照；取消与
/// 确认回调直接 `update` 实体，改名成功后实体自行关闭弹窗，不再经外壳转发。
pub(crate) fn render_container_rename_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    _p: &Palette,
) -> AnyElement {
    let Some((_, input)) = state.rename_container.clone() else {
        return div().into_any_element();
    };

    let content = v_flex().gap_3().child(Input::new(&input).w_full());

    let cancel = dialog_cancel_button(
        "cancel-container-rename",
        rust_i18n::t!("common.cancel").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.cancel_container_rename(cx));
            }
        },
    );

    let confirm = dialog_confirm_button(
        "confirm-container-rename",
        rust_i18n::t!("common.confirm").to_string(),
        Some(IconName::Check),
        DialogConfirmKind::Primary,
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.submit_container_rename(cx));
            }
        },
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    dialog_body(
        "container-rename-modal",
        rust_i18n::t!("container.rename.modal_title").to_string(),
        Some(rust_i18n::t!("container.rename.prompt").to_string()),
        content,
        Some(footer),
    )
}
