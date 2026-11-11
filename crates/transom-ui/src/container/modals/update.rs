//! 容器资源限制更新居中模态弹窗。

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

/// 渲染容器资源限制更新模态弹窗主体。
///
/// 设计原理（Why）：两个输入控件与打开状态都归工作区实体，渲染只读状态快照；
/// 取消与确认回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_update_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let Some((_, mem_input, cpu_input)) = state.update_container.clone() else {
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
                            rust_i18n::t!("container.update_resources.memory_label").to_string(),
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
                        .child(rust_i18n::t!("container.update_resources.cpu_label").to_string()),
                )
                .child(Input::new(&cpu_input).w_full()),
        );

    let cancel = dialog_cancel_button(
        "cancel-container-update",
        rust_i18n::t!("common.cancel").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.cancel_container_update(cx));
            }
        },
    );

    let confirm = dialog_confirm_button(
        "confirm-container-update",
        rust_i18n::t!("common.confirm").to_string(),
        Some(AppIcon::Ui(UiIcon::CHECK)),
        DialogConfirmKind::Primary,
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.submit_container_update(cx));
            }
        },
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    dialog_body(
        "container-update-modal",
        rust_i18n::t!("container.update_resources.modal_title").to_string(),
        Some(rust_i18n::t!("container.update_resources.prompt").to_string()),
        content,
        Some(footer),
    )
}
