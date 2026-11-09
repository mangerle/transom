//! 容器文件管理居中模态弹窗（重命名与新建目录）。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

/// 渲染容器文件重命名模态弹窗主体。
///
/// 设计原理（Why）：输入框与提交/取消动作属于工作区实体，回调直接
/// `update` 实体；渲染函数只读状态快照，不反查外壳。
pub(crate) fn render_container_file_rename_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    _p: &Palette,
) -> AnyElement {
    let Some(input) = state.files.prompt_input.clone() else {
        return div().into_any_element();
    };
    let Some(prompt) = state.files.prompt.clone() else {
        return div().into_any_element();
    };

    let content = v_flex().gap_3().child(Input::new(&input).w_full());

    let cancel = dialog_cancel_button(
        "cancel-container-file-rename",
        rust_i18n::t!("common.cancel").to_string(),
        crate::dialog_host::on_event(view, {
            move |workspace, _: &ClickEvent, _, cx| {
                workspace.cancel_container_file_rename(cx);
            }
        }),
    );

    let confirm = dialog_confirm_button(
        "confirm-container-file-rename",
        rust_i18n::t!("common.confirm").to_string(),
        Some(IconName::Check),
        DialogConfirmKind::Primary,
        crate::dialog_host::on_event(view, {
            move |workspace, _: &ClickEvent, _, cx| {
                workspace.submit_container_file_prompt(cx);
            }
        }),
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    let title = rust_i18n::t!(prompt.title_key()).to_string();
    let prompt_text = rust_i18n::t!(prompt.prompt_key()).to_string();
    dialog_body(
        "container-file-rename-modal",
        title,
        Some(prompt_text),
        content,
        Some(footer),
    )
}

/// 渲染容器新建目录模态弹窗主体。
pub(crate) fn render_container_file_new_dir_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    _p: &Palette,
) -> AnyElement {
    let Some(input) = state.files.new_dir_input.clone() else {
        return div().into_any_element();
    };

    let content = v_flex().gap_3().child(Input::new(&input).w_full());

    let cancel = dialog_cancel_button(
        "cancel-container-file-mkdir",
        rust_i18n::t!("common.cancel").to_string(),
        crate::dialog_host::on_event(view, {
            move |workspace, _: &ClickEvent, _, cx| {
                workspace.cancel_container_file_new_dir(cx);
            }
        }),
    );

    let confirm = dialog_confirm_button(
        "confirm-container-file-mkdir",
        rust_i18n::t!("container.create").to_string(),
        Some(IconName::Plus),
        DialogConfirmKind::Primary,
        crate::dialog_host::on_event(view, {
            move |workspace, _: &ClickEvent, _, cx| {
                workspace.submit_container_mkdir(cx);
            }
        }),
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    dialog_body(
        "container-file-mkdir-modal",
        rust_i18n::t!("container.files.mkdir_modal_title").to_string(),
        Some(rust_i18n::t!("container.files.mkdir_prompt").to_string()),
        content,
        Some(footer),
    )
}
