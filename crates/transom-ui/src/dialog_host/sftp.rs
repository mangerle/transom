//! SFTP 模态弹窗的渲染分发。

use super::ModalKind;
use crate::app::AppState;
use gpui_kit::*;

impl ModalKind {
    /// 渲染由 SFTP 弹窗集合持有的模态内容。
    pub(super) fn render_sftp_modal_body(self, state: &AppState) -> Option<AnyElement> {
        let sftp = &state.ui.modals.sftp;
        let body = match self {
            ModalKind::SftpConfirm => sftp
                .confirm
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            ModalKind::SftpInput => sftp
                .input
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            ModalKind::SftpChmod => sftp
                .chmod
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            ModalKind::SftpProperties => sftp
                .properties
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            ModalKind::SftpFilter => sftp
                .filter
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            ModalKind::SftpSearch => sftp
                .search
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            ModalKind::SftpSync => sftp
                .sync
                .as_ref()
                .map(|modal| modal.clone().into_any_element()),
            _ => return None,
        };
        Some(body.unwrap_or_else(|| div().into_any_element()))
    }
}
