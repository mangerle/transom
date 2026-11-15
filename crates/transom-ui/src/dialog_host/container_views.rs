//! 容器弹窗的初始焦点与弹窗体渲染分派。

//! 容器类模态弹窗的状态判定、提交、取消与焦点分发。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::AppShell;
use crate::container::ContainerWorkspaceState;
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// 容器弹窗首帧焦点控件。
    pub(super) fn container_focus_target(self, shell: &AppShell, cx: &App) -> Option<ModalFocus> {
        let focus = match self {
            ModalKind::ContainerRename => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.rename_container.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.rename_container.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerTagImage => {
                crate::container::find_container_modal_tab(shell, cx, |ws| ws.tag_image.is_some())
                    .and_then(|(_, ws)| ws.read(cx).state.tag_image.as_ref())
                    .map(|(_, input)| ModalFocus::Input(input.clone()))
                    .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerCommit => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.commit_container.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.commit_container.as_ref())
                .map(|state| ModalFocus::Input(state.repo.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerUpdate => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.update_container.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.update_container.as_ref())
                .map(|(_, mem, _)| ModalFocus::Input(mem.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerNetworkConnect => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.connect_network.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.connect_network.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerFileRename => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.files.prompt_input.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.files.prompt_input.as_ref())
                .map(|input| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerFileNewDir => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.files.new_dir_input.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.files.new_dir_input.as_ref())
                .map(|input| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerHub => {
                crate::container::find_container_modal_tab(shell, cx, |ws| ws.hub_dialog.is_some())
                    .and_then(|(_, ws)| ws.read(cx).state.hub_dialog.as_ref())
                    .map(|hub| ModalFocus::Input(hub.search_input.clone()))
                    .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerPush => {
                crate::container::find_container_modal_tab(shell, cx, |ws| ws.push_dialog.is_some())
                    .and_then(|(_, ws)| ws.read(cx).state.push_dialog.as_ref())
                    .map(|push| ModalFocus::Input(push.target_repo.clone()))
                    .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerBuild => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.build_dialog.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.build_dialog.as_ref())
                .map(|build| ModalFocus::Input(build.context_path.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerForward => {
                crate::container::find_container_modal_tab(shell, cx, |ws| {
                    ws.forward_dialog.is_some()
                })
                .and_then(|(_, ws)| ws.read(cx).state.forward_dialog.as_ref())
                .map(|fwd| ModalFocus::Input(fwd.local_port_input.clone()))
                .unwrap_or(ModalFocus::None)
            }
            ModalKind::ContainerConfirm
            | ModalKind::ContainerExportConfig
            | ModalKind::ContainerSystemDf
            | ModalKind::ContainerPods
            | ModalKind::ContainerQuadlet => ModalFocus::None,
            _ => return None,
        };
        Some(focus)
    }
}

impl ModalKind {
    /// 渲染容器工作区实体持有的模态弹窗主体。
    ///
    /// 设计原理（Why）：弹窗状态归各标签页的 [`ContainerWorkspace`]，渲染函数
    /// 只接收实体句柄、只读状态快照与 `Palette`；取消/确认回调直接 `update`
    /// 实体。宿主只负责按种类找到当前打开该弹窗的工作区并注入参数，
    /// 不再把 `Context<AppShell>` 传进任何容器弹窗体。
    ///
    /// 返回 `None` 表示这个种类不是容器工作区实体弹窗，交回宿主继续分发。
    pub(super) fn container_workspace_body(
        self,
        shell: &AppShell,
        cx: &App,
        p: &Palette,
    ) -> Option<AnyElement> {
        let predicate: fn(&ContainerWorkspaceState) -> bool = match self {
            ModalKind::ContainerConfirm => |state| state.confirm.is_some(),
            ModalKind::ContainerRename => |state| state.rename_container.is_some(),
            ModalKind::ContainerTagImage => |state| state.tag_image.is_some(),
            ModalKind::ContainerCommit => |state| state.commit_container.is_some(),
            ModalKind::ContainerUpdate => |state| state.update_container.is_some(),
            ModalKind::ContainerNetworkConnect => |state| state.connect_network.is_some(),
            ModalKind::ContainerExportConfig => |state| state.export_config.is_some(),
            ModalKind::ContainerFileRename => |state| state.files.prompt_input.is_some(),
            ModalKind::ContainerFileNewDir => {
                |state| state.files.new_dir_open && state.files.new_dir_input.is_some()
            }
            ModalKind::ContainerForward => |state| state.forward_dialog.is_some(),
            ModalKind::ContainerPush => |state| state.push_dialog.is_some(),
            ModalKind::ContainerSystemDf => |state| state.system_df_dialog_open,
            ModalKind::ContainerBuild => |state| state.build_dialog.is_some(),
            ModalKind::ContainerHub => |state| state.hub_dialog.is_some(),
            ModalKind::ContainerPods => |state| state.pods_dialog_open,
            ModalKind::ContainerQuadlet => |state| state.quadlets_dialog_open,
            _ => return None,
        };
        let (_, view) = crate::container::find_container_modal_tab(shell, cx, predicate)?;
        let state = view.read(cx);
        let element = match self {
            ModalKind::ContainerConfirm => {
                crate::container::render_container_confirm_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerRename => {
                crate::container::render_container_rename_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerTagImage => {
                crate::container::render_container_tag_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerCommit => {
                crate::container::render_container_commit_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerUpdate => {
                crate::container::render_container_update_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerNetworkConnect => {
                crate::container::render_container_network_connect_dialog_body(
                    &view,
                    &state.state,
                    p,
                )
            }
            ModalKind::ContainerExportConfig => {
                crate::container::render_container_export_config_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerFileRename => {
                crate::container::render_container_file_rename_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerFileNewDir => {
                crate::container::render_container_file_new_dir_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerForward => {
                crate::container::render_container_forward_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerPush => {
                let registries = crate::app::app_settings(cx).container_registries.clone();
                crate::container::render_container_push_dialog_body(
                    &view,
                    &state.state,
                    &registries,
                    p,
                )
            }
            ModalKind::ContainerSystemDf => {
                crate::container::render_container_system_df_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerBuild => {
                crate::container::render_container_build_dialog_body(&view, &state.state, cx, p)
            }
            ModalKind::ContainerHub => {
                crate::container::render_container_hub_dialog_body(&view, &state.state, cx, p)
            }
            ModalKind::ContainerPods => {
                crate::container::render_container_pods_dialog_body(&view, &state.state, p)
            }
            ModalKind::ContainerQuadlet => {
                crate::container::render_container_quadlet_dialog_body(&view, &state.state, p)
            }
            _ => return None,
        };
        Some(element)
    }
}
