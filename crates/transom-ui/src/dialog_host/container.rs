//! 容器类模态弹窗的状态判定、提交、取消与焦点分发。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::AppShell;
use crate::container::ContainerWorkspaceState;
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// 容器弹窗取消动作。若属于容器弹窗种类，执行取消并返回 `Some(true)`；否则返回 `None`。
    pub(super) fn container_cancel(
        self,
        shell: &mut AppShell,
        cx: &mut Context<AppShell>,
    ) -> Option<bool> {
        match self {
            ModalKind::ContainerConfirm => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.confirm.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_confirm(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerRename => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_rename(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerTagImage => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_image_tag(cx));
                }
                Some(true)
            }
            ModalKind::ContainerCommit => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_commit(cx));
                }
                Some(true)
            }
            ModalKind::ContainerUpdate => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.update_container.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_update(cx));
                }
                Some(true)
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_network_connect(cx));
                }
                Some(true)
            }
            ModalKind::ContainerExportConfig => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.export_config.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_export(cx));
                }
                Some(true)
            }
            ModalKind::ContainerFileRename => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.files.prompt_input.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_file_rename(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerFileNewDir => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.files.new_dir_input.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_file_new_dir(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerHub => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.close_hub_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerPush => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_push_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerSystemDf => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.system_df_dialog_open)
                {
                    workspace.update(cx, |workspace, cx| workspace.close_system_df_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerBuild => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.close_build_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerForward => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_port_forward(cx)
                    });
                }
                Some(true)
            }
            ModalKind::ContainerPods => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.pods_dialog_open)
                {
                    workspace.update(cx, |workspace, cx| workspace.close_pods_dialog(cx));
                }
                Some(true)
            }
            _ => None,
        }
    }

    /// 容器弹窗提交动作。若属于容器弹窗种类，执行提交并返回 `Some(())`；否则返回 `None`。
    pub(super) fn container_commit(
        self,
        shell: &mut AppShell,
        _window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<()> {
        match self {
            ModalKind::ContainerConfirm => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.confirm.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.apply_container_confirm(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerRename => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_rename(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerTagImage => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_image_tag(cx));
                }
                Some(())
            }
            ModalKind::ContainerCommit => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_container_commit(cx));
                }
                Some(())
            }
            ModalKind::ContainerUpdate => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.update_container.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_container_update(cx));
                }
                Some(())
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_network_connect(cx));
                }
                Some(())
            }
            ModalKind::ContainerExportConfig => {
                // 导出弹窗没有“提交”：确认键与关闭键同义，直接关闭。
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.export_config.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_export(cx));
                }
                Some(())
            }
            ModalKind::ContainerFileRename => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.files.prompt_input.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_file_prompt(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerFileNewDir => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.files.new_dir_input.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_mkdir(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerHub => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.execute_hub_search(cx));
                }
                Some(())
            }
            ModalKind::ContainerPush => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_push_dialog(cx));
                }
                Some(())
            }
            ModalKind::ContainerBuild => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_build_dialog(cx));
                }
                Some(())
            }
            ModalKind::ContainerForward => {
                if let Some((_, workspace)) =
                    shell.find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_port_forward(cx)
                    });
                }
                Some(())
            }
            ModalKind::ContainerPods | ModalKind::ContainerQuadlet => Some(()),
            _ => None,
        }
    }

    /// 容器弹窗首帧焦点控件。
    pub(super) fn container_focus_target(self, shell: &AppShell, cx: &App) -> Option<ModalFocus> {
        let focus = match self {
            ModalKind::ContainerRename => shell
                .find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.rename_container.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerTagImage => shell
                .find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.tag_image.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerCommit => shell
                .find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.commit_container.as_ref())
                .map(|state| ModalFocus::Input(state.repo.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerUpdate => shell
                .find_container_modal_tab(cx, |ws| ws.update_container.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.update_container.as_ref())
                .map(|(_, mem, _)| ModalFocus::Input(mem.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerNetworkConnect => shell
                .find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.connect_network.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerFileRename => shell
                .find_container_modal_tab(cx, |ws| ws.files.prompt_input.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.files.prompt_input.as_ref())
                .map(|input| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerFileNewDir => shell
                .find_container_modal_tab(cx, |ws| ws.files.new_dir_input.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.files.new_dir_input.as_ref())
                .map(|input| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerHub => shell
                .find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.hub_dialog.as_ref())
                .map(|hub| ModalFocus::Input(hub.search_input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerPush => shell
                .find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.push_dialog.as_ref())
                .map(|push| ModalFocus::Input(push.target_repo.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerBuild => shell
                .find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.build_dialog.as_ref())
                .map(|build| ModalFocus::Input(build.context_path.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerForward => shell
                .find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
                .and_then(|(_, ws)| ws.read(cx).state.forward_dialog.as_ref())
                .map(|fwd| ModalFocus::Input(fwd.local_port_input.clone()))
                .unwrap_or(ModalFocus::None),
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
        let (_, view) = shell.find_container_modal_tab(cx, predicate)?;
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
