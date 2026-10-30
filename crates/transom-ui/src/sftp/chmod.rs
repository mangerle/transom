//! SFTP 文件权限操作：远端 Chmod 弹窗的打开、位切换与保存，以及右键一键授予执行位。

use super::SftpChmodModalState;
use crate::app::AppShell;
use crate::palette::sftp_text_input;
use crate::sftp::SftpRemotePaneRef;
use gpui_kit::*;
use transom_core::{SftpOperation, TabId, format_octal_permissions, parse_octal_permissions};

/// 在现有权限上叠加属主/组/其他三方的执行位，其余位（含 setuid、sticky）原样保留。
///
/// 设计原理（Why）：
/// - 实现初衷：口径对齐 `chmod +x`。上传脚本后要跑，用户要的是"能执行"而不是"重写整串权限"，
///   所以只能在原值上或出执行位；
/// - 核心优势：直接写死 755 会把组写位、特殊位一并覆盖，`u+x` 又常在别人执行时失败，
///   按位叠加对两种误操作都免疫；服务端未回权限时退回目录 755、文件 644，
///   与 Chmod 弹窗打开时用的基线（`sftp_open_chmod_modal`）保持同一套约定；
/// - 代价与局限：这是"只加不减"，已经带执行位的条目再点一次结果不变，不提供撤销入口——
///   减位属于精细调整，交给 Chmod 弹窗。
fn exec_granted_mode(current: Option<u32>, is_dir: bool) -> u32 {
    let baseline = current.unwrap_or(if is_dir { 0o755 } else { 0o644 }) & 0o7777;
    baseline | 0o111
}

impl AppShell {
    /// 远端右键一键授予执行位：对当前定位到的选中项逐个下发 Chmod 操作。
    ///
    /// 目标集合的取法与"下载""删除"完全一致：右键项已在多选集里就整批处理，
    /// 否则只处理它自己，避免同一面板里出现两套批量语义。
    /// 目标远端列一键授予执行位：主远端与并列远端共用同一套批量口径。
    pub(crate) fn sftp_grant_remote_exec_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some((connection_id, targets)) = self
            .remote_pane_scope(tab_id, target)
            .map(|scope| (scope.connection_id, scope.pane.selected_paths()))
        else {
            return;
        };
        let mut plans: Vec<(String, u32)> = Vec::with_capacity(targets.len());
        if let Some(panel) = self.workspace.tab(tab_id).and_then(|tab| tab.sftp()) {
            let Some(pane) = panel.remote_pane(target) else {
                return;
            };
            for path in targets {
                let entry = pane.entries.iter().find(|e| e.path == path);
                let mode = exec_granted_mode(
                    entry.and_then(|e| e.permissions),
                    entry.is_some_and(|e| e.is_dir),
                );
                plans.push((path, mode));
            }
        }
        if plans.is_empty() {
            self.set_sftp_pane_error(
                tab_id,
                target,
                rust_i18n::t!("sftp.select_file").to_string(),
                cx,
            );
            return;
        }
        let count = plans.len();
        let dir = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| panel.remote_pane(target))
            .map(|pane| pane.path.clone())
            .unwrap_or_default();
        for (path, permissions) in plans {
            self.send_sftp_op_for_connection(
                tab_id,
                connection_id,
                SftpOperation::Chmod { path, permissions },
                cx,
            );
        }
        log::info!("远端赋予执行权限已下发，窗格: {tab_id}，条目数: {count}，目录: {dir}");
        // 状态只声明"已下发"：Chmod 是异步往返，此刻列表里的权限徽章尚未更新，
        // 说"已生效"就是在骗用户；刷新目录后即可看到新值。
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
            panel.set_status(rust_i18n::t!("sftp.chmod.granted_remote", count = count).to_string());
            cx.notify();
        }
    }

    /// 本地右键一键授予执行位（仅 Unix）。
    ///
    /// Windows 文件系统没有执行位可言，故本函数在非 Unix 平台根本不参与编译，
    /// 菜单项也同步隐藏——宁可少一个入口，不给一个注定失败的按钮。
    #[cfg(unix)]
    pub(crate) fn sftp_grant_local_exec(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        let targets: Vec<PathBuf> = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|t| t.sftp_mut())
            .map(|panel| panel.selected_local_paths())
            .unwrap_or_default();
        if targets.is_empty() {
            if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
                panel.set_local_error(rust_i18n::t!("sftp.select_file").to_string());
                cx.notify();
            }
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

        let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) else {
            return;
        };
        if let Some(first) = failures.first() {
            panel.set_local_error(
                rust_i18n::t!(
                    "sftp.chmod.grant_failed",
                    failed = failures.len(),
                    total = targets.len(),
                    first = first.as_str()
                )
                .to_string(),
            );
        } else {
            panel
                .set_status(rust_i18n::t!("sftp.chmod.granted_local", count = granted).to_string());
        }
        cx.notify();
    }

    /// 打开目标远端列的 Chmod 弹窗。
    pub(crate) fn sftp_open_chmod_modal_for(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((entry, side_connection)) =
            self.remote_pane_scope(tab_id, target).and_then(|scope| {
                let selected = scope.pane.selected.clone()?;
                let entry = scope
                    .pane
                    .entries
                    .iter()
                    .find(|entry| entry.path == selected)?
                    .clone();
                Some((entry, scope.side_connection))
            })
        else {
            self.set_sftp_pane_error(
                tab_id,
                target,
                rust_i18n::t!("sftp.select_file").to_string(),
                cx,
            );
            return;
        };
        let default_mode = if entry.is_dir { 0o755 } else { 0o644 };
        let permissions = entry.permissions.unwrap_or(default_mode) & 0o777;
        let octal_str = format_octal_permissions(permissions);
        let octal_input = sftp_text_input(
            window,
            cx,
            &octal_str,
            &rust_i18n::t!("sftp.chmod.octal_placeholder"),
        );
        octal_input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.chmod = Some(SftpChmodModalState {
            tab_id,
            connection_id: side_connection,
            remote_path: entry.path,
            file_name: entry.name,
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
                    modal.error = Some(rust_i18n::t!("sftp.chmod.octal_invalid").to_string());
                    let input = modal.octal_input.clone();
                    self.sftp_modals.chmod = Some(modal);
                    input.update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                    return;
                }
            }
        };
        let tab_id = modal.tab_id;
        let Some(connection_id) = modal
            .connection_id
            .or_else(|| self.workspace.tab(tab_id).and_then(|tab| tab.connection_id))
        else {
            return;
        };
        let op = SftpOperation::Chmod {
            path: modal.remote_path,
            permissions: target_perms,
        };
        self.send_sftp_op_for_connection(tab_id, connection_id, op, cx);
        self.focus_active_sftp_panel(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::exec_granted_mode;

    #[test]
    fn adds_exec_bits_without_touching_others() {
        assert_eq!(exec_granted_mode(Some(0o644), false), 0o755);
        assert_eq!(
            exec_granted_mode(Some(0o664), false),
            0o775,
            "组写位必须保留"
        );
        assert_eq!(
            exec_granted_mode(Some(0o600), false),
            0o711,
            "口径是 +x 而非 +X：没有读位的主体同样拿到执行位"
        );
    }

    #[test]
    fn keeps_special_bits_and_stays_idempotent() {
        assert_eq!(
            exec_granted_mode(Some(0o4755), false),
            0o4755,
            "setuid 位不应被抹掉"
        );
        assert_eq!(exec_granted_mode(Some(0o2644), false), 0o2755);
        assert_eq!(
            exec_granted_mode(Some(0o755), false),
            0o755,
            "重复点击结果不变"
        );
    }

    #[test]
    fn falls_back_to_chmod_modal_baseline_when_server_omits_permissions() {
        assert_eq!(exec_granted_mode(None, true), 0o755);
        assert_eq!(
            exec_granted_mode(None, false),
            0o755,
            "文件按 644 打底再或上执行位"
        );
    }

    #[test]
    fn masks_type_bits_out_of_incoming_mode() {
        // 本地路径传入的是 st_mode（含 0o100000 文件类型位），叠加后只保留权限位。
        assert_eq!(exec_granted_mode(Some(0o100644), false), 0o755);
        assert_eq!(exec_granted_mode(Some(0o40755), true), 0o755);
    }
}
