//! 网络连接容器居中模态弹窗。

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

/// 渲染网络连接容器模态弹窗主体。
///
/// 设计原理（Why）：输入控件与打开状态都归工作区实体，渲染只读状态快照；
/// 取消与确认回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_network_connect_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    _p: &Palette,
) -> AnyElement {
    let Some((_, input)) = state.connect_network.clone() else {
        return div().into_any_element();
    };

    let content = v_flex().gap_3().child(Input::new(&input).w_full());

    let cancel = dialog_cancel_button(
        "cancel-network-connect",
        rust_i18n::t!("common.cancel").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.cancel_network_connect(cx));
            }
        },
    );

    let confirm = dialog_confirm_button(
        "confirm-network-connect",
        rust_i18n::t!("common.confirm").to_string(),
        Some(AppIcon::Ui(UiIcon::CHECK)),
        DialogConfirmKind::Primary,
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.submit_network_connect(cx));
            }
        },
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    dialog_body(
        "network-connect-modal",
        rust_i18n::t!("container.network_connect.modal_title").to_string(),
        Some(rust_i18n::t!("container.network_connect.prompt").to_string()),
        content,
        Some(footer),
    )
}
