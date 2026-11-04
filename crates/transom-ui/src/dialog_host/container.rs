//! 容器类模态弹窗的状态判定、提交、取消与焦点分发。

use super::body::ModalFocus;
use super::ModalKind;
use crate::app::AppShell;
use gpui_kit::*;

impl ModalKind {
    /// 容器弹窗是否处于打开状态。若属于容器弹窗种类，返回 `Some(bool)`；否则返回 `None`。
    pub(super) fn container_is_open(self, shell: &AppShell) -> Option<bool> {
        let is_open = match self {
            ModalKind::ContainerConfirm => shell
                .find_container_modal_tab(|ws| ws.confirm.is_some())
                .is_some(),
            ModalKind::ContainerRename => shell
                .find_container_modal_tab(|ws| ws.rename_container.is_some())
                .is_some(),
            ModalKind::ContainerTagImage => shell
                .find_container_modal_tab(|ws| ws.tag_image.is_some())
                .is_some(),
            ModalKind::ContainerCommit => shell
                .find_container_modal_tab(|ws| ws.commit_container.is_some())
                .is_some(),
            ModalKind::ContainerUpdate => shell
                .find_container_modal_tab(|ws| ws.update_container.is_some())
                .is_some(),
            ModalKind::ContainerNetworkConnect => shell
                .find_container_modal_tab(|ws| ws.connect_network.is_some())
                .is_some(),
            ModalKind::ContainerExportConfig => shell
                .find_container_modal_tab(|ws| ws.export_config.is_some())
                .is_some(),
            ModalKind::ContainerFileRename => shell
                .find_container_modal_tab(|ws| ws.files.rename_input.is_some())
                .is_some(),
            ModalKind::ContainerFileNewDir => shell
                .find_container_modal_tab(|ws| {
                    ws.files.new_dir_open && ws.files.new_dir_input.is_some()
                })
                .is_some(),
            _ => return None,
        };
        Some(is_open)
    }

    /// 容器弹窗取消动作。若属于容器弹窗种类，执行取消并返回 `Some(true)`；否则返回 `None`。
    pub(super) fn container_cancel(
        self,
        shell: &mut AppShell,
        cx: &mut Context<AppShell>,
    ) -> Option<bool> {
        match self {
            ModalKind::ContainerConfirm => {
                if let Some((tab_id, _)) = shell.find_container_modal_tab(|ws| ws.confirm.is_some())
                {
                    shell.cancel_container_confirm(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerRename => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.rename_container.is_some())
                {
                    shell.cancel_container_rename(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerTagImage => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.tag_image.is_some())
                {
                    shell.cancel_image_tag(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerCommit => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.commit_container.is_some())
                {
                    shell.cancel_container_commit(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerUpdate => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.update_container.is_some())
                {
                    shell.cancel_container_update(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.connect_network.is_some())
                {
                    shell.cancel_network_connect(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerExportConfig => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.export_config.is_some())
                {
                    shell.cancel_container_export(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerFileRename => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.files.rename_input.is_some())
                {
                    shell.cancel_container_file_rename(tab_id, cx);
                }
                Some(true)
            }
            ModalKind::ContainerFileNewDir => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.files.new_dir_input.is_some())
                {
                    shell.cancel_container_file_new_dir(tab_id, cx);
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
                    shell.find_container_modal_tab(|ws| ws.confirm.is_some())
                {
                    if let Some(confirm) = ws.confirm.clone() {
                        shell.apply_container_confirm(tab_id, confirm, cx);
                    }
                }
                Some(())
            }
            ModalKind::ContainerRename => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.rename_container.is_some())
                {
                    shell.submit_container_rename(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerTagImage => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.tag_image.is_some())
                {
                    shell.submit_image_tag(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerCommit => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.commit_container.is_some())
                {
                    shell.submit_container_commit(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerUpdate => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.update_container.is_some())
                {
                    shell.submit_container_update(tab_id, cx);
                }
                Some(())
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.connect_network.is_some())
                {
                    shell.submit_network_connect(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerExportConfig => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.export_config.is_some())
                {
                    shell.cancel_container_export(tab_id, cx);
                }
                Some(())
            }
            ModalKind::ContainerFileRename => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.files.rename_input.is_some())
                {
                    shell.submit_container_file_rename(tab_id, window, cx);
                }
                Some(())
            }
            ModalKind::ContainerFileNewDir => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.files.new_dir_input.is_some())
                {
                    shell.submit_container_mkdir(tab_id, window, cx);
                }
                Some(())
            }
            _ => None,
        }
    }

    /// 容器弹窗首帧焦点控件。
    pub(super) fn container_focus_target(self, shell: &AppShell) -> Option<ModalFocus> {
        let focus = match self {
            ModalKind::ContainerRename => shell
                .find_container_modal_tab(|ws| ws.rename_container.is_some())
                .and_then(|(_, ws)| ws.rename_container.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerTagImage => shell
                .find_container_modal_tab(|ws| ws.tag_image.is_some())
                .and_then(|(_, ws)| ws.tag_image.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerCommit => shell
                .find_container_modal_tab(|ws| ws.commit_container.is_some())
                .and_then(|(_, ws)| ws.commit_container.as_ref())
                .map(|(_, repo, _, _)| ModalFocus::Input(repo.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerUpdate => shell
                .find_container_modal_tab(|ws| ws.update_container.is_some())
                .and_then(|(_, ws)| ws.update_container.as_ref())
                .map(|(_, mem, _)| ModalFocus::Input(mem.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerNetworkConnect => shell
                .find_container_modal_tab(|ws| ws.connect_network.is_some())
                .and_then(|(_, ws)| ws.connect_network.as_ref())
                .map(|(_, input)| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerFileRename => shell
                .find_container_modal_tab(|ws| ws.files.rename_input.is_some())
                .and_then(|(_, ws)| ws.files.rename_input.as_ref())
                .map(|input| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerFileNewDir => shell
                .find_container_modal_tab(|ws| ws.files.new_dir_input.is_some())
                .and_then(|(_, ws)| ws.files.new_dir_input.as_ref())
                .map(|input| ModalFocus::Input(input.clone()))
                .unwrap_or(ModalFocus::None),
            ModalKind::ContainerConfirm | ModalKind::ContainerExportConfig => ModalFocus::None,
            _ => return None,
        };
        Some(focus)
    }

    /// 容器弹窗文本字段追踪。
    pub(super) fn container_field_values(
        self,
        shell: &AppShell,
        cx: &App,
    ) -> Option<Vec<String>> {
        let values = match self {
            ModalKind::ContainerRename => shell
                .find_container_modal_tab(|ws| ws.rename_container.is_some())
                .and_then(|(_, ws)| ws.rename_container.as_ref())
                .map(|(_, input)| vec![input.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::ContainerTagImage => shell
                .find_container_modal_tab(|ws| ws.tag_image.is_some())
                .and_then(|(_, ws)| ws.tag_image.as_ref())
                .map(|(_, input)| vec![input.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::ContainerCommit => shell
                .find_container_modal_tab(|ws| ws.commit_container.is_some())
                .and_then(|(_, ws)| ws.commit_container.as_ref())
                .map(|(_, repo, tag, comment)| {
                    vec![
                        repo.read(cx).value().to_string(),
                        tag.read(cx).value().to_string(),
                        comment.read(cx).value().to_string(),
                    ]
                })
                .unwrap_or_default(),
            ModalKind::ContainerUpdate => shell
                .find_container_modal_tab(|ws| ws.update_container.is_some())
                .and_then(|(_, ws)| ws.update_container.as_ref())
                .map(|(_, mem, cpu)| {
                    vec![
                        mem.read(cx).value().to_string(),
                        cpu.read(cx).value().to_string(),
                    ]
                })
                .unwrap_or_default(),
            ModalKind::ContainerNetworkConnect => shell
                .find_container_modal_tab(|ws| ws.connect_network.is_some())
                .and_then(|(_, ws)| ws.connect_network.as_ref())
                .map(|(_, input)| vec![input.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::ContainerFileRename => shell
                .find_container_modal_tab(|ws| ws.files.rename_input.is_some())
                .and_then(|(_, ws)| ws.files.rename_input.as_ref())
                .map(|input| vec![input.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::ContainerFileNewDir => shell
                .find_container_modal_tab(|ws| ws.files.new_dir_input.is_some())
                .and_then(|(_, ws)| ws.files.new_dir_input.as_ref())
                .map(|input| vec![input.read(cx).value().to_string()])
                .unwrap_or_default(),
            ModalKind::ContainerConfirm | ModalKind::ContainerExportConfig => Vec::new(),
            _ => return None,
        };
        Some(values)
    }
}
