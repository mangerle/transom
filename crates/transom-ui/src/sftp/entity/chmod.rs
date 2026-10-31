//! 实体侧的权限操作：一键授予执行位与打开 Chmod 弹窗。
//!
//! 权限计算与选中集都基于面板状态；弹窗内的位切换与保存仍由对话框提交路径
//! 处理，因为那一步持有窗口级弹窗状态。

use super::*;
use crate::palette::sftp_text_input;
use crate::sftp::chmod::exec_granted_mode;
use crate::sftp::{SftpChmodModalState, SftpRemotePaneRef};
use transom_core::{SftpOperation, format_octal_permissions};

impl SftpPanelView {
    /// 远端右键一键授予执行位：对目标列选中项逐个下发 Chmod。
    pub(crate) fn grant_remote_exec(&mut self, target: SftpRemotePaneRef, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let Some(main_connection) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(self.tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            return;
        };
        let Some(pane) = self.state.remote_pane(target) else {
            return;
        };
        let targets = pane.selected_paths();
        let mut plans: Vec<(String, u32)> = Vec::with_capacity(targets.len());
        for path in targets {
            let entry = pane.entries.iter().find(|entry| entry.path == path);
            let mode = exec_granted_mode(
                entry.and_then(|entry| entry.permissions),
                entry.is_some_and(|entry| entry.is_dir),
            );
            plans.push((path, mode));
        }
        if plans.is_empty() {
            self.set_pane_error(target, rust_i18n::t!("sftp.select_file").to_string(), cx);
            return;
        }
        let count = plans.len();
        let dir = pane.path.clone();
        for (path, permissions) in plans {
            self.send_op_for_connection(
                connection_id,
                SftpOperation::Chmod { path, permissions },
                cx,
            );
        }
        log::info!(
            "远端赋予执行权限已下发，窗格: {}，条目数: {count}，目录: {dir}",
            self.tab_id
        );
        // 状态只声明"已下发"：Chmod 是异步往返，此刻列表里的权限徽章尚未更新。
        self.state
            .set_status(rust_i18n::t!("sftp.chmod.granted_remote", count = count).to_string());
        cx.notify();
    }

    /// 本地右键一键授予执行位（仅 Unix）。
    #[cfg(unix)]
    pub(crate) fn grant_local_exec(&mut self, cx: &mut Context<Self>) {
        use std::os::unix::fs::PermissionsExt;

        let targets = self.state.selected_local_paths();
        if targets.is_empty() {
            self.state
                .set_local_error(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }

        let mut granted = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for path in &targets {
            let outcome = std::fs::metadata(path)
                .map(|meta| meta.permissions().mode())
                .and_then(|mode| {
                    let next = exec_granted_mode(Some(mode), path.is_dir());
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(next))
                });
            match outcome {
                Ok(()) => granted = granted.saturating_add(1),
                Err(error) => {
                    log::error!(
                        "本地赋予执行权限失败，路径: {}，原因: {error}",
                        path.display()
                    );
                    failures.push(path.display().to_string());
                }
            }
        }

        if let Some(first) = failures.first() {
            self.state.set_local_error(
                rust_i18n::t!(
                    "sftp.chmod.grant_failed",
                    failed = failures.len(),
                    total = targets.len(),
                    first = first.as_str()
                )
                .to_string(),
            );
        } else {
            self.state
                .set_status(rust_i18n::t!("sftp.chmod.granted_local", count = granted).to_string());
        }
        cx.notify();
    }

    /// 打开目标远端列的 Chmod 弹窗。
    pub(crate) fn open_chmod_modal(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.state.remote_pane(target).and_then(|pane| {
            let selected = pane.selected.clone()?;
            pane.entries
                .iter()
                .find(|entry| entry.path == selected)
                .cloned()
        }) else {
            self.set_pane_error(target, rust_i18n::t!("sftp.select_file").to_string(), cx);
            return;
        };
        let side_connection = match target {
            SftpRemotePaneRef::Main => None,
            SftpRemotePaneRef::Side(connection_id) => Some(connection_id),
        };
        let default_mode = if entry.is_dir { 0o755 } else { 0o644 };
        let permissions = entry.permissions.unwrap_or(default_mode) & 0o777;
        let octal_str = format_octal_permissions(permissions);
        let placeholder = rust_i18n::t!("sftp.chmod.octal_placeholder").to_string();
        let tab_id = self.tab_id;
        let app = self.app.clone();
        with_shell_parts(&app, cx, |shell, shell_cx| {
            let octal_input = sftp_text_input(window, shell_cx, &octal_str, &placeholder);
            octal_input.update(shell_cx, |state, cx| state.focus(window, cx));
            shell.state.sftp_modals.chmod = Some(SftpChmodModalState {
                tab_id,
                connection_id: side_connection,
                remote_path: entry.path,
                file_name: entry.name,
                permissions,
                octal_input,
                error: None,
            });
            shell_cx.notify();
        });
    }
}
