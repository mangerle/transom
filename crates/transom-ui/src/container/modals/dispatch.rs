//! 容器工作区实体弹窗的统一分发契约。
//!
//! 设计原理（Why）：
//! - 容器工作区共持有 16 类局部弹窗；过去宿主层（`dialog_host`）不得不在
//!   判定打开态、取首帧焦点、取消、提交与渲染主体 5 处各写 16 遍
//!   `find_container_modal_tab` + `match`，造成数百行重复样板与跨层字段穿透；
//! - 将 16 类弹窗收敛为 [`ContainerWorkspaceModal`] 枚举后，宿主只负责将
//!   `ModalKind` 映射为本枚举并定位工作区实体，其余生命周期闭环全部内聚在
//!   容器领域内部。

use super::{
    render_container_commit_dialog_body, render_container_confirm_dialog_body,
    render_container_export_config_dialog_body, render_container_file_new_dir_dialog_body,
    render_container_file_rename_dialog_body, render_container_forward_dialog_body,
    render_container_network_connect_dialog_body, render_container_pods_dialog_body,
    render_container_quadlet_dialog_body, render_container_rename_dialog_body,
    render_container_tag_dialog_body, render_container_update_dialog_body,
};
use crate::app::app_settings;
use crate::container::{
    ContainerWorkspace, ContainerWorkspaceState, render_container_build_dialog_body,
    render_container_hub_dialog_body, render_container_push_dialog_body,
    render_container_system_df_dialog_body,
};
use crate::palette::Palette;
use gpui_kit::component::input::InputState;
use gpui_kit::*;

/// 容器工作区实体持有的模态弹窗种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerWorkspaceModal {
    Confirm,
    Rename,
    TagImage,
    Commit,
    Update,
    NetworkConnect,
    ExportConfig,
    FileRename,
    FileNewDir,
    Hub,
    Push,
    SystemDf,
    Build,
    Forward,
    Pods,
    Quadlet,
}

impl ContainerWorkspaceModal {
    /// 判断指定弹窗在当前工作区状态中是否处于打开态。
    pub(crate) fn is_open(self, state: &ContainerWorkspaceState) -> bool {
        match self {
            Self::Confirm => state.confirm.is_some(),
            Self::Rename => state.rename_container.is_some(),
            Self::TagImage => state.tag_image.is_some(),
            Self::Commit => state.commit_container.is_some(),
            Self::Update => state.update_container.is_some(),
            Self::NetworkConnect => state.connect_network.is_some(),
            Self::ExportConfig => state.export_config.is_some(),
            Self::FileRename => state.files.prompt_input.is_some(),
            Self::FileNewDir => state.files.new_dir_open && state.files.new_dir_input.is_some(),
            Self::Hub => state.hub_dialog.is_some(),
            Self::Push => state.push_dialog.is_some(),
            Self::SystemDf => state.system_df_dialog_open,
            Self::Build => state.build_dialog.is_some(),
            Self::Forward => state.forward_dialog.is_some(),
            Self::Pods => state.pods_dialog_open,
            Self::Quadlet => state.quadlets_dialog_open,
        }
    }

    /// 返回弹窗首帧应聚焦的输入控件；无输入框的展示类弹窗返回 `None`。
    pub(crate) fn focus_input(self, state: &ContainerWorkspaceState) -> Option<Entity<InputState>> {
        match self {
            Self::Rename => state
                .rename_container
                .as_ref()
                .map(|(_, input)| input.clone()),
            Self::TagImage => state.tag_image.as_ref().map(|(_, input)| input.clone()),
            Self::Commit => state
                .commit_container
                .as_ref()
                .map(|commit| commit.repo.clone()),
            Self::Update => state.update_container.as_ref().map(|(_, mem, _)| mem.clone()),
            Self::NetworkConnect => state
                .connect_network
                .as_ref()
                .map(|(_, input)| input.clone()),
            Self::FileRename => state.files.prompt_input.clone(),
            Self::FileNewDir => state.files.new_dir_input.clone(),
            Self::Hub => state.hub_dialog.as_ref().map(|hub| hub.search_input.clone()),
            Self::Push => state
                .push_dialog
                .as_ref()
                .map(|push| push.target_repo.clone()),
            Self::Build => state
                .build_dialog
                .as_ref()
                .map(|build| build.context_path.clone()),
            Self::Forward => state
                .forward_dialog
                .as_ref()
                .map(|fwd| fwd.local_port_input.clone()),
            Self::Confirm
            | Self::ExportConfig
            | Self::SystemDf
            | Self::Pods
            | Self::Quadlet => None,
        }
    }

    /// 取消或关闭该工作区弹窗。
    pub(crate) fn cancel(
        self,
        workspace: &mut ContainerWorkspace,
        cx: &mut Context<ContainerWorkspace>,
    ) {
        match self {
            Self::Confirm => workspace.cancel_container_confirm(cx),
            Self::Rename => workspace.cancel_container_rename(cx),
            Self::TagImage => workspace.cancel_image_tag(cx),
            Self::Commit => workspace.cancel_container_commit(cx),
            Self::Update => workspace.cancel_container_update(cx),
            Self::NetworkConnect => workspace.cancel_network_connect(cx),
            Self::ExportConfig => workspace.cancel_container_export(cx),
            Self::FileRename => workspace.cancel_container_file_rename(cx),
            Self::FileNewDir => workspace.cancel_container_file_new_dir(cx),
            Self::Hub => workspace.close_hub_dialog(cx),
            Self::Push => workspace.cancel_push_dialog(cx),
            Self::SystemDf => workspace.close_system_df_dialog(cx),
            Self::Build => workspace.close_build_dialog(cx),
            Self::Forward => workspace.cancel_container_port_forward(cx),
            Self::Pods => workspace.close_pods_dialog(cx),
            Self::Quadlet => workspace.close_quadlets_dialog(cx),
        }
    }

    /// 提交该工作区弹窗的主按钮动作。
    pub(crate) fn commit(
        self,
        workspace: &mut ContainerWorkspace,
        cx: &mut Context<ContainerWorkspace>,
    ) {
        match self {
            Self::Confirm => workspace.apply_container_confirm(cx),
            Self::Rename => workspace.submit_container_rename(cx),
            Self::TagImage => workspace.submit_image_tag(cx),
            Self::Commit => workspace.submit_container_commit(cx),
            Self::Update => workspace.submit_container_update(cx),
            Self::NetworkConnect => workspace.submit_network_connect(cx),
            // 导出弹窗没有“提交”：确认键与关闭键同义，直接关闭。
            Self::ExportConfig => workspace.cancel_container_export(cx),
            Self::FileRename => workspace.submit_container_file_prompt(cx),
            Self::FileNewDir => workspace.submit_container_mkdir(cx),
            Self::Hub => workspace.execute_hub_search(cx),
            Self::Push => workspace.submit_push_dialog(cx),
            Self::Build => workspace.submit_build_dialog(cx),
            Self::Forward => workspace.submit_container_port_forward(cx),
            Self::SystemDf | Self::Pods | Self::Quadlet => {}
        }
    }

    /// 渲染弹窗主体内容。
    pub(crate) fn render_body(
        self,
        view: &Entity<ContainerWorkspace>,
        state: &ContainerWorkspaceState,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        match self {
            Self::Confirm => render_container_confirm_dialog_body(view, state, p),
            Self::Rename => render_container_rename_dialog_body(view, state, p),
            Self::TagImage => render_container_tag_dialog_body(view, state, p),
            Self::Commit => render_container_commit_dialog_body(view, state, p),
            Self::Update => render_container_update_dialog_body(view, state, p),
            Self::NetworkConnect => render_container_network_connect_dialog_body(view, state, p),
            Self::ExportConfig => render_container_export_config_dialog_body(view, state, p),
            Self::FileRename => render_container_file_rename_dialog_body(view, state, p),
            Self::FileNewDir => render_container_file_new_dir_dialog_body(view, state, p),
            Self::Forward => render_container_forward_dialog_body(view, state, p),
            Self::Push => {
                let registries = app_settings(cx).container_registries.clone();
                render_container_push_dialog_body(view, state, &registries, p)
            }
            Self::SystemDf => render_container_system_df_dialog_body(view, state, p),
            Self::Build => render_container_build_dialog_body(view, state, cx, p),
            Self::Hub => render_container_hub_dialog_body(view, state, cx, p),
            Self::Pods => render_container_pods_dialog_body(view, state, p),
            Self::Quadlet => render_container_quadlet_dialog_body(view, state, p),
        }
    }
}
