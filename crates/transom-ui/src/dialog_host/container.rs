//! 容器类模态弹窗的状态判定、提交、取消与焦点分发。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::AppShell;
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
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.confirm.is_some())
                {
                    shell.cancel_container_confirm(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerRename => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
                {
                    shell.cancel_container_rename(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerTagImage => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
                {
                    shell.cancel_image_tag(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerCommit => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
                {
                    shell.cancel_container_commit(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerUpdate => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.update_container.is_some())
                {
                    shell.cancel_container_update(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
                {
                    shell.cancel_network_connect(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerExportConfig => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.export_config.is_some())
                {
                    shell.cancel_container_export(tab_id, cx);
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
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
                {
                    shell.with_container_mut(tab_id, cx, |ws| ws.hub_dialog = None);
                    cx.notify();
                }
                Some(true)
            }
            ModalKind::ContainerPush => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
                {
                    shell.with_container_mut(tab_id, cx, |ws| ws.push_dialog = None);
                    cx.notify();
                }
                Some(true)
            }
            ModalKind::ContainerSystemDf => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.system_df_dialog_open)
                {
                    shell.with_container_mut(tab_id, cx, |ws| ws.system_df_dialog_open = false);
                    cx.notify();
                }
                Some(true)
            }
            ModalKind::ContainerBuild => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
                {
                    shell.with_container_mut(tab_id, cx, |ws| ws.build_dialog = None);
                    cx.notify();
                }
                Some(true)
            }
            ModalKind::ContainerForward => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
                {
                    shell.cancel_container_port_forward(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerPods => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.pods_dialog_open)
                {
                    shell.close_container_pods_dialog(tab_id, cx);
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
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<()> {
        match self {
            ModalKind::ContainerConfirm => {
                if let Some((tab_id, ws)) =
                    shell.find_container_modal_tab(cx, |ws| ws.confirm.is_some())
                    && let Some(confirm) = ws.read(cx).state.confirm.clone()
                {
                    shell.apply_container_confirm(tab_id, confirm, cx);
                }
                Some(())
            }
            ModalKind::ContainerRename => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
                {
                    shell.submit_container_rename(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerTagImage => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
                {
                    shell.submit_image_tag(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerCommit => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
                {
                    shell.submit_container_commit(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerUpdate => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.update_container.is_some())
                {
                    shell.submit_container_update(tab_id, cx);
                }
                Some(())
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
                {
                    shell.submit_network_connect(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerExportConfig => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.export_config.is_some())
                {
                    shell.cancel_container_export(tab_id, cx);
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
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
                {
                    shell.execute_hub_search(tab_id, cx);
                }
                Some(())
            }
            ModalKind::ContainerPush => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
                {
                    shell.submit_container_push_dialog(tab_id, cx);
                }
                Some(())
            }
            ModalKind::ContainerBuild => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
                {
                    shell.submit_container_build_dialog(tab_id, cx);
                }
                Some(())
            }
            ModalKind::ContainerForward => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
                {
                    shell.submit_container_port_forward(tab_id, cx);
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
