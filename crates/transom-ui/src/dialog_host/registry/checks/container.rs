//! 容器类全局弹窗的判定与文本项。
//!
//! 16 类隶属于 [`crate::container::ContainerWorkspace`] 的局部弹窗已全部收束到
//! `ContainerWorkspaceModal`，由 `dialog_host/state/mod.rs` 通过
//! `as_container_modal()` 统一分发，本文件不再逐字段探测工作区内部状态。

use super::super::DialogCheckState;
use gpui_kit::App;

pub(in crate::app::dialog_host) fn container_form_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.tools.container_form.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn container_form_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state.modals.tools.container_form.read(cx).text_values(cx)
}

pub(in crate::app::dialog_host) fn container_folder_form_open(
    state: &DialogCheckState,
    cx: &App,
) -> bool {
    state
        .containers
        .sidebar
        .read(cx)
        .state
        .engine_folder_form
        .is_some()
}

pub(in crate::app::dialog_host) fn container_folder_form_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .containers
        .sidebar
        .read(cx)
        .state
        .engine_folder_form
        .as_ref()
        .map(|form| vec![form.input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_run_form_open(
    state: &DialogCheckState,
    cx: &App,
) -> bool {
    state.modals.tools.run_form.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn container_run_form_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state.modals.tools.run_form.read(cx).text_values(cx)
}

pub(in crate::app::dialog_host) fn container_create_open(
    state: &DialogCheckState,
    cx: &App,
) -> bool {
    state.modals.tools.create_dialog.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn container_create_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    let view = state.modals.tools.create_dialog.read(cx);
    let Some(dialog) = view.state() else {
        return Vec::new();
    };
    let mut values = Vec::with_capacity(4);
    if let Some(workspace) = state
        .workspace
        .tab(dialog.tab_id)
        .and_then(|tab| tab.container_workspace())
    {
        values.push(
            workspace
                .read(cx)
                .state
                .create_input
                .read(cx)
                .value()
                .to_string(),
        );
    }
    values.extend(view.text_values(cx));
    values
}

/// 工作区局部弹窗的注册表占位判定；实际判定由 `ModalKind::as_container_modal` 拦截接管。
pub(in crate::app::dialog_host) fn workspace_modal_placeholder(
    _: &DialogCheckState,
    _: &App,
) -> bool {
    false
}
