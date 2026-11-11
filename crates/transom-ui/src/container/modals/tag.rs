//! 镜像添加标签居中模态弹窗。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 渲染镜像打标签模态弹窗主体。
///
/// 设计原理（Why）：输入控件与打开状态都归工作区实体，渲染只读状态快照；
/// 取消与确认回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_tag_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    _p: &Palette,
) -> AnyElement {
    let Some((_, input)) = state.tag_image.clone() else {
        return div().into_any_element();
    };

    let content = v_flex().gap_3().child(Input::new(&input).w_full());

    let cancel = dialog_cancel_button(
        "cancel-image-tag",
        rust_i18n::t!("common.cancel").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.cancel_image_tag(cx));
            }
        },
    );

    let confirm = dialog_confirm_button(
        "confirm-image-tag",
        rust_i18n::t!("common.confirm").to_string(),
        Some(AppIcon::Ui(UiIcon::CHECK)),
        DialogConfirmKind::Primary,
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.submit_image_tag(cx));
            }
        },
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    dialog_body(
        "image-tag-modal",
        rust_i18n::t!("container.tag.modal_title").to_string(),
        Some(rust_i18n::t!("container.tag.prompt").to_string()),
        content,
        Some(footer),
    )
}
