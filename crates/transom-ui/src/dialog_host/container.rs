//! 容器类模态弹窗的取消、提交、焦点与弹窗体分发。
//!
//! 设计原理（Why）：弹窗状态与渲染细节全部内聚在 `container` 领域的
//! [`ContainerWorkspaceModal`] 中，宿主只负责将 [`ModalKind`] 映射为容器工作区
//! 弹窗种类并定位当前打开该弹窗的工作区实体，不再直接读取或枚举工作区状态字段。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::{AppShell, AppState};
use crate::container::{ContainerFormFolders, ContainerWorkspaceModal};
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// 将宿主弹窗种类映射为容器工作区弹窗种类；不属于工作区弹窗时返回 `None`。
    pub(super) fn as_container_modal(self) -> Option<ContainerWorkspaceModal> {
        let kind = match self {
            ModalKind::ContainerConfirm => ContainerWorkspaceModal::Confirm,
            ModalKind::ContainerRename => ContainerWorkspaceModal::Rename,
            ModalKind::ContainerTagImage => ContainerWorkspaceModal::TagImage,
            ModalKind::ContainerCommit => ContainerWorkspaceModal::Commit,
            ModalKind::ContainerUpdate => ContainerWorkspaceModal::Update,
            ModalKind::ContainerNetworkConnect => ContainerWorkspaceModal::NetworkConnect,
            ModalKind::ContainerExportConfig => ContainerWorkspaceModal::ExportConfig,
            ModalKind::ContainerFileRename => ContainerWorkspaceModal::FileRename,
            ModalKind::ContainerFileNewDir => ContainerWorkspaceModal::FileNewDir,
            ModalKind::ContainerHub => ContainerWorkspaceModal::Hub,
            ModalKind::ContainerPush => ContainerWorkspaceModal::Push,
            ModalKind::ContainerSystemDf => ContainerWorkspaceModal::SystemDf,
            ModalKind::ContainerBuild => ContainerWorkspaceModal::Build,
            ModalKind::ContainerForward => ContainerWorkspaceModal::Forward,
            ModalKind::ContainerPods => ContainerWorkspaceModal::Pods,
            ModalKind::ContainerQuadlet => ContainerWorkspaceModal::Quadlet,
            _ => return None,
        };
        Some(kind)
    }

    /// 容器弹窗取消动作。若属于容器弹窗种类，执行取消并返回 `Some(true)`；否则返回 `None`。
    pub(super) fn container_cancel(
        self,
        app_state: &mut AppState,
        cx: &mut Context<AppShell>,
    ) -> Option<bool> {
        let modal = self.as_container_modal()?;
        if let Some((_, workspace)) =
            crate::container::find_container_modal_tab(app_state, cx, |ws| modal.is_open(ws))
        {
            workspace.update(cx, |workspace, cx| modal.cancel(workspace, cx));
        }
        Some(true)
    }

    /// 容器弹窗提交动作。若属于容器弹窗种类，执行提交并返回 `Some(())`；否则返回 `None`。
    pub(super) fn container_commit(
        self,
        app_state: &mut AppState,
        _window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<()> {
        let modal = self.as_container_modal()?;
        if let Some((_, workspace)) =
            crate::container::find_container_modal_tab(app_state, cx, |ws| modal.is_open(ws))
        {
            workspace.update(cx, |workspace, cx| modal.commit(workspace, cx));
        }
        Some(())
    }

    /// 容器弹窗首帧焦点控件。
    pub(super) fn container_focus_target(
        self,
        app_state: &AppState,
        cx: &App,
    ) -> Option<ModalFocus> {
        let modal = self.as_container_modal()?;
        let focus =
            crate::container::find_container_modal_tab(app_state, cx, |ws| modal.is_open(ws))
                .and_then(|(_, ws)| modal.focus_input(&ws.read(cx).state))
                .map_or(ModalFocus::None, ModalFocus::Input);
        Some(focus)
    }

    /// 渲染容器工作区实体持有的模态弹窗主体。
    ///
    /// 返回 `None` 表示这个种类不是容器工作区实体弹窗，交回宿主继续分发。
    pub(super) fn container_workspace_body(
        self,
        app_state: &AppState,
        cx: &App,
        p: &Palette,
    ) -> Option<AnyElement> {
        let modal = self.as_container_modal()?;
        let (_, view) =
            crate::container::find_container_modal_tab(app_state, cx, |ws| modal.is_open(ws))?;
        let state = &view.read(cx).state;
        Some(modal.render_body(&view, state, cx, p))
    }

    /// 容器引擎连接表单主体。
    pub(super) fn container_form_body(
        self,
        app: &Entity<AppShell>,
        cx: &mut App,
        p: &Palette,
    ) -> Option<AnyElement> {
        if self != ModalKind::ContainerForm {
            return None;
        }
        let (view, folders) = {
            let app_state = &app.read(cx).state;
            let options = app_state
                .session
                .catalog
                .engine_folders
                .iter()
                .map(|folder| (folder.id, folder.name.clone()))
                .collect();
            (
                app_state.ui.modals.tools.container_form.clone(),
                ContainerFormFolders { options },
            )
        };
        Some(view.update(cx, |view, cx| view.render_body(&folders, p, cx)))
    }

    /// 运行容器表单主体，镜像摘要由外壳快照注入。
    pub(super) fn container_run_form_body(
        self,
        app: &Entity<AppShell>,
        cx: &mut App,
        p: &Palette,
    ) -> Option<AnyElement> {
        if self != ModalKind::ContainerRunForm {
            return None;
        }
        let (view, image_meta) = {
            let app_state = &app.read(cx).state;
            let view = app_state.ui.modals.tools.run_form.clone();
            let image_meta = view.read(cx).state().and_then(|form| {
                let image_raw = form.image.read(cx).value().trim().to_string();
                app_state
                    .container_workspace(form.tab_id)
                    .and_then(|workspace| {
                        workspace
                            .read(cx)
                            .state
                            .images
                            .iter()
                            .find(|image| {
                                image.tags.contains(&image_raw) || image.id.starts_with(&image_raw)
                            })
                            .cloned()
                    })
            });
            (view, image_meta)
        };
        Some(view.update(cx, |view, cx| view.render_body(image_meta.as_ref(), p, cx)))
    }

    /// 新建容器/卷/网络弹窗主体，主输入仍归目标工作区。
    pub(super) fn container_create_body(
        self,
        app: &Entity<AppShell>,
        cx: &mut App,
        p: &Palette,
    ) -> Option<AnyElement> {
        if self != ModalKind::ContainerCreate {
            return None;
        }
        let (view, create_input) = {
            let app_state = &app.read(cx).state;
            let view = app_state.ui.modals.tools.create_dialog.clone();
            let create_input = view.read(cx).state().and_then(|dialog| {
                app_state
                    .container_workspace(dialog.tab_id)
                    .map(|workspace| workspace.read(cx).state.create_input.clone())
            });
            (view, create_input)
        };
        let Some(create_input) = create_input else {
            return Some(div().into_any_element());
        };
        Some(view.update(cx, |view, cx| view.render_body(create_input, p, cx)))
    }
}
