//! SFTP 远端文件权限 (Chmod) 弹窗的打开、位切换与保存逻辑。

use super::SftpChmodModalState;
use crate::app::AppShell;
use crate::palette::sftp_text_input;
use gpui_kit::*;
use transom_core::{SftpOperation, TabId, format_octal_permissions, parse_octal_permissions};

impl AppShell {
    pub(crate) fn sftp_open_chmod_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        let Some(selected) = panel.selected.clone() else {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        };
        let Some(entry) = panel.entries.iter().find(|e| e.path == selected) else {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        };
        let default_mode = if entry.is_dir { 0o755 } else { 0o644 };
        let permissions = entry.permissions.unwrap_or(default_mode) & 0o777;
        let octal_str = format_octal_permissions(permissions);
        let octal_input = sftp_text_input(window, cx, &octal_str, "例如 755 或 644");
        octal_input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.chmod = Some(SftpChmodModalState {
            tab_id,
            remote_path: entry.path.clone(),
            file_name: entry.name.clone(),
            permissions,
            octal_input,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn sftp_toggle_chmod_bit(
        &mut self,
        mask: u32,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.sftp_modals.chmod.as_mut() else {
            return;
        };
        if checked {
            modal.permissions |= mask;
        } else {
            modal.permissions &= !mask;
        }
        modal.error = None;
        let octal_str = format_octal_permissions(modal.permissions);
        let input = modal.octal_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(octal_str, window, cx);
        });
        cx.notify();
    }

    pub(crate) fn sftp_set_chmod_preset(
        &mut self,
        mode: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.sftp_modals.chmod.as_mut() else {
            return;
        };
        modal.permissions = mode & 0o777;
        modal.error = None;
        let octal_str = format_octal_permissions(modal.permissions);
        let input = modal.octal_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(octal_str, window, cx);
        });
        cx.notify();
    }

    pub(crate) fn save_sftp_chmod_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut modal) = self.sftp_modals.chmod.take() else {
            return;
        };
        let input_text = modal.octal_input.read(cx).value().trim().to_string();
        let target_perms = if input_text.is_empty() {
            modal.permissions & 0o777
        } else {
            match parse_octal_permissions(&input_text) {
                Some(p) => p,
                None => {
                    modal.error =
                        Some("无效的八进制权限码（请输入 000 - 777 之间的八进制数字）".to_string());
                    let input = modal.octal_input.clone();
                    self.sftp_modals.chmod = Some(modal);
                    input.update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                    return;
                }
            }
        };
        let tab_id = modal.tab_id;
        let op = SftpOperation::Chmod {
            path: modal.remote_path,
            permissions: target_perms,
        };
        self.send_sftp_op(tab_id, op, cx);
        self.focus_active_sftp_panel(window, cx);
    }
}
