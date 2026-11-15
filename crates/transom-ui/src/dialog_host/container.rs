//! 容器类模态弹窗的状态判定、提交、取消与焦点分发。

use super::ModalKind;
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
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| ws.confirm.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_confirm(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerRename => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.rename_container.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_rename(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerTagImage => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.tag_image.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_image_tag(cx));
                }
                Some(true)
            }
            ModalKind::ContainerCommit => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.commit_container.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_commit(cx));
                }
                Some(true)
            }
            ModalKind::ContainerUpdate => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.update_container.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_update(cx));
                }
                Some(true)
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.connect_network.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_network_connect(cx));
                }
                Some(true)
            }
            ModalKind::ContainerExportConfig => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.export_config.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_export(cx));
                }
                Some(true)
            }
            ModalKind::ContainerFileRename => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.files.prompt_input.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_file_rename(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerFileNewDir => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.files.new_dir_input.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_file_new_dir(cx);
                    });
                }
                Some(true)
            }
            ModalKind::ContainerHub => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.hub_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.close_hub_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerPush => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.push_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_push_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerSystemDf => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.system_df_dialog_open
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.close_system_df_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerBuild => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.build_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.close_build_dialog(cx));
                }
                Some(true)
            }
            ModalKind::ContainerForward => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.forward_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.cancel_container_port_forward(cx)
                    });
                }
                Some(true)
            }
            ModalKind::ContainerPods => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| ws.pods_dialog_open)
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
                    crate::container::find_container_modal_tab(shell, cx, |ws| ws.confirm.is_some())
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.apply_container_confirm(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerRename => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.rename_container.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_rename(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerTagImage => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.tag_image.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_image_tag(cx));
                }
                Some(())
            }
            ModalKind::ContainerCommit => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.commit_container.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_container_commit(cx));
                }
                Some(())
            }
            ModalKind::ContainerUpdate => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.update_container.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_container_update(cx));
                }
                Some(())
            }
            ModalKind::ContainerNetworkConnect => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.connect_network.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_network_connect(cx));
                }
                Some(())
            }
            ModalKind::ContainerExportConfig => {
                // 导出弹窗没有“提交”：确认键与关闭键同义，直接关闭。
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.export_config.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.cancel_container_export(cx));
                }
                Some(())
            }
            ModalKind::ContainerFileRename => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.files.prompt_input.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_file_prompt(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerFileNewDir => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.files.new_dir_input.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| {
                        workspace.submit_container_mkdir(cx);
                    });
                }
                Some(())
            }
            ModalKind::ContainerHub => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.hub_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.execute_hub_search(cx));
                }
                Some(())
            }
            ModalKind::ContainerPush => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.push_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_push_dialog(cx));
                }
                Some(())
            }
            ModalKind::ContainerBuild => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.build_dialog.is_some()
                    })
                {
                    workspace.update(cx, |workspace, cx| workspace.submit_build_dialog(cx));
                }
                Some(())
            }
            ModalKind::ContainerForward => {
                if let Some((_, workspace)) =
                    crate::container::find_container_modal_tab(shell, cx, |ws| {
                        ws.forward_dialog.is_some()
                    })
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
}
