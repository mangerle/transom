//! 容器类模态弹窗的取消、提交、焦点与弹窗体分发。
//!
//! 设计原理（Why）：弹窗状态与渲染细节全部内聚在 `container` 领域的
//! [`ContainerWorkspaceModal`] 中，宿主只负责将 [`ModalKind`] 映射为容器工作区
//! 弹窗种类并定位当前打开该弹窗的工作区实体，不再直接读取或枚举工作区状态字段。

use super::ModalKind;
use super::body::ModalFocus;
use crate::app::AppShell;
use crate::container::ContainerWorkspaceModal;
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
        shell: &mut AppShell,
        cx: &mut Context<AppShell>,
    ) -> Option<bool> {
        let modal = self.as_container_modal()?;
        if let Some((_, workspace)) =
            crate::container::find_container_modal_tab(shell, cx, |ws| modal.is_open(ws))
        {
            workspace.update(cx, |workspace, cx| modal.cancel(workspace, cx));
        }
        Some(true)
    }

    /// 容器弹窗提交动作。若属于容器弹窗种类，执行提交并返回 `Some(())`；否则返回 `None`。
    pub(super) fn container_commit(
        self,
        shell: &mut AppShell,
        _window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<()> {
        let modal = self.as_container_modal()?;
        if let Some((_, workspace)) =
            crate::container::find_container_modal_tab(shell, cx, |ws| modal.is_open(ws))
        {
            workspace.update(cx, |workspace, cx| modal.commit(workspace, cx));
        }
        Some(())
    }

    /// 容器弹窗首帧焦点控件。
    pub(super) fn container_focus_target(self, shell: &AppShell, cx: &App) -> Option<ModalFocus> {
        let modal = self.as_container_modal()?;
        let focus = crate::container::find_container_modal_tab(shell, cx, |ws| modal.is_open(ws))
            .and_then(|(_, ws)| modal.focus_input(&ws.read(cx).state))
            .map_or(ModalFocus::None, ModalFocus::Input);
        Some(focus)
    }

    /// 渲染容器工作区实体持有的模态弹窗主体。
    ///
    /// 返回 `None` 表示这个种类不是容器工作区实体弹窗，交回宿主继续分发。
    pub(super) fn container_workspace_body(
        self,
        shell: &AppShell,
        cx: &App,
        p: &Palette,
    ) -> Option<AnyElement> {
        let modal = self.as_container_modal()?;
        let (_, view) =
            crate::container::find_container_modal_tab(shell, cx, |ws| modal.is_open(ws))?;
        let state = &view.read(cx).state;
        Some(modal.render_body(&view, state, cx, p))
    }
}
