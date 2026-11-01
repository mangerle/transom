//! SFTP 模态输入框提交处理与本地软链接创建。

use super::validate::name_validation_error;
use crate::app::AppShell;
use crate::sftp::{
    SftpInputTarget, SftpRemotePaneRef, join_remote, parent_remote, spawn_blocking_local,
};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{SftpOperation, TabId};

impl AppShell {
    /// 提交当前打开的 SFTP 输入模态对话框，执行创建文件夹、新建文件编辑、重命名或软链接。
    pub(crate) fn commit_sftp_input_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.state.sftp_modals.input.take() else {
            return;
        };
        let tab_id = modal.tab_id;
        let name = modal.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            let mut modal = modal;
            modal.error = Some(rust_i18n::t!("sftp.file_ops.name_empty").to_string());
            let input = modal.input.clone();
            self.state.sftp_modals.input = Some(modal);
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if let Some(message) = name_validation_error(&modal.target, &name) {
            let mut modal = modal;
            modal.error = Some(message);
            let input = modal.input.clone();
            self.state.sftp_modals.input = Some(modal);
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        // 远端目标的连接与当前目录：并列远端弹窗携带目标连接，主远端沿用
        // 标签页连接。所有远端操作与后续刷新都按这里解析出的连接路由。
        let target = match modal.connection_id {
            Some(connection_id) => SftpRemotePaneRef::Side(connection_id),
            None => SftpRemotePaneRef::Main,
        };
        let remote_scope = self.remote_pane_scope(tab_id, target, cx).map(|scope| {
            (
                scope.connection_id,
                scope.pane.path.clone(),
                scope.pane.entries.clone(),
            )
        });

        // 新建文件在通用校验后直接分流：目录列表里已有同名条目时不创建，
        // 目录报错回填、文件直接打开编辑；全新路径才生成空编辑面板。
        // 整条分支在 match 之前完成，避免部分移动的模态再被改写。
        if matches!(modal.target, SftpInputTarget::RemoteEditorNewFile) {
            let Some((connection_id, base_path, entries)) = remote_scope else {
                return;
            };
            let path = join_remote(&base_path, &name);
            let existing_is_dir = entries
                .iter()
                .find(|entry| entry.path == path)
                .map(|entry| entry.is_dir);
            match existing_is_dir {
                Some(true) => {
                    let mut modal = modal;
                    modal.error = Some(
                        rust_i18n::t!("sftp.file_ops.file_exists_as_dir", name = name.as_str())
                            .to_string(),
                    );
                    let input = modal.input.clone();
                    self.state.sftp_modals.input = Some(modal);
                    input.update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                }
                // 列表里已有同名文件：交给既有打开流程（读取失败会在面板内报错）。
                Some(false) => {
                    self.sftp_edit_remote_file_on(tab_id, connection_id, path, window, cx)
                }
                // 目录列表未命中：按新文件打开空编辑面板。
                None => self.spawn_new_editor_panel(tab_id, connection_id, path, name, window, cx),
            }
            return;
        }

        match modal.target {
            SftpInputTarget::RemoteMkdir => {
                let Some((connection_id, base_path, _)) = remote_scope else {
                    return;
                };
                let op = SftpOperation::Mkdir {
                    path: join_remote(&base_path, &name),
                };
                self.send_sftp_op_for_connection(tab_id, connection_id, op, cx);
            }
            // 新建文件已在 match 之前的前置分支完整处理：同名条目分流、
            // 目录报错回填、文件直接打开或生成空面板。
            SftpInputTarget::RemoteEditorNewFile => {}
            SftpInputTarget::RemoteRename { old_path } => {
                let Some((connection_id, _, _)) = remote_scope else {
                    return;
                };
                let op = SftpOperation::Rename {
                    from: old_path.clone(),
                    to: join_remote(&parent_remote(&old_path), &name),
                };
                self.send_sftp_op_for_connection(tab_id, connection_id, op, cx);
            }
            SftpInputTarget::RemoteSymlink { target } => {
                let Some((connection_id, base_path, _)) = remote_scope else {
                    return;
                };
                let op = SftpOperation::Symlink {
                    target,
                    link_path: join_remote(&base_path, &name),
                };
                self.send_sftp_op_for_connection(tab_id, connection_id, op, cx);
            }
            SftpInputTarget::RemoteArchive {
                source_paths,
                parent_dir,
            } => {
                let Some((connection_id, _, _)) = remote_scope else {
                    return;
                };
                let dest_archive_path = join_remote(&parent_dir, &name);
                let op = SftpOperation::Archive {
                    source_paths,
                    dest_archive_path,
                };
                self.send_sftp_op_for_connection(tab_id, connection_id, op, cx);
            }
            SftpInputTarget::RemoteExtract {
                archive_path,
                parent_dir,
            } => {
                let Some((connection_id, _, _)) = remote_scope else {
                    return;
                };
                let dest_dir = join_remote(&parent_dir, &name);
                let op = SftpOperation::Extract {
                    archive_path,
                    dest_dir,
                };
                self.send_sftp_op_for_connection(tab_id, connection_id, op, cx);
            }
            SftpInputTarget::SudoElevation { editor } => {
                let password = if name.trim().is_empty() {
                    None
                } else {
                    Some(name)
                };
                self.submit_sudo_editor_write(&editor, password, window, cx);
            }
            SftpInputTarget::LocalMkdir => {
                let Some(local_path) = self
                    .sftp_state(tab_id, cx)
                    .map(|panel| panel.local.path.clone())
                else {
                    return;
                };
                let op = LocalFsOp::Mkdir(local_path.join(&name));
                self.spawn_local_fs(tab_id, op, cx);
            }
            SftpInputTarget::LocalNewFile => {
                let Some(local_path) = self
                    .sftp_state(tab_id, cx)
                    .map(|panel| panel.local.path.clone())
                else {
                    return;
                };
                let op = LocalFsOp::NewFile(local_path.join(&name));
                self.spawn_local_fs(tab_id, op, cx);
            }
            SftpInputTarget::LocalRename { old_path } => {
                let Some(parent) = old_path.parent() else {
                    if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
                        panel.state.set_local_error(
                            rust_i18n::t!(
                                "sftp.file_ops.local_rename_failed",
                                error = rust_i18n::t!("sftp.file_ops.no_parent_dir")
                            )
                            .to_string(),
                        );
                    }
                    cx.notify();
                    return;
                };
                let op = LocalFsOp::Rename(old_path.clone(), parent.join(&name));
                self.spawn_local_fs(tab_id, op, cx);
            }
            SftpInputTarget::LocalSymlink { target_path } => {
                let Some(local_path) = self
                    .sftp_state(tab_id, cx)
                    .map(|panel| panel.local.path.clone())
                else {
                    return;
                };
                let op = LocalFsOp::Symlink(target_path, local_path.join(&name));
                self.spawn_local_fs(tab_id, op, cx);
            }
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }
}

/// 本地文件操作：在后台线程执行，成功后回到 UI 线程刷新。
enum LocalFsOp {
    Mkdir(PathBuf),
    NewFile(PathBuf),
    Rename(PathBuf, PathBuf),
    Symlink(PathBuf, PathBuf),
}

/// 本地文件操作结果；字段都是已本地化文案或待选中路径。
struct LocalFsOutcome {
    status: Option<String>,
    error: Option<String>,
    select: Option<PathBuf>,
    reveal: Option<PathBuf>,
}

fn run_local_fs(op: LocalFsOp) -> LocalFsOutcome {
    match op {
        LocalFsOp::Mkdir(path) => match std::fs::create_dir_all(&path) {
            Ok(()) => LocalFsOutcome {
                status: Some(
                    rust_i18n::t!(
                        "sftp.file_ops.local_mkdir_created",
                        name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned()
                        )
                    )
                    .to_string(),
                ),
                error: None,
                select: None,
                reveal: None,
            },
            Err(error) => LocalFsOutcome {
                status: None,
                error: Some(
                    rust_i18n::t!("sftp.file_ops.local_mkdir_failed", error = error).to_string(),
                ),
                select: None,
                reveal: None,
            },
        },
        LocalFsOp::NewFile(path) => {
            if path.exists() {
                return LocalFsOutcome {
                    status: None,
                    error: Some(
                        rust_i18n::t!("sftp.file_ops.local_file_exists", name = path.display())
                            .to_string(),
                    ),
                    select: None,
                    reveal: None,
                };
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => LocalFsOutcome {
                    status: Some(
                        rust_i18n::t!("sftp.file_ops.local_file_created", name = path.display())
                            .to_string(),
                    ),
                    error: None,
                    select: Some(path.clone()),
                    reveal: Some(path),
                },
                Err(error) => LocalFsOutcome {
                    status: None,
                    error: Some(
                        rust_i18n::t!("sftp.file_ops.local_file_create_failed", error = error)
                            .to_string(),
                    ),
                    select: None,
                    reveal: None,
                },
            }
        }
        LocalFsOp::Rename(from, to) => match std::fs::rename(&from, &to) {
            Ok(()) => LocalFsOutcome {
                status: Some(
                    rust_i18n::t!("sftp.file_ops.local_renamed", name = to.display()).to_string(),
                ),
                error: None,
                select: Some(to),
                reveal: None,
            },
            Err(error) => LocalFsOutcome {
                status: None,
                error: Some(
                    rust_i18n::t!("sftp.file_ops.local_rename_failed", error = error).to_string(),
                ),
                select: None,
                reveal: None,
            },
        },
        LocalFsOp::Symlink(target, link) => {
            #[cfg(unix)]
            let result =
                std::os::unix::fs::symlink(&target, &link).map_err(|error| error.to_string());
            #[cfg(not(unix))]
            let result = Err(rust_i18n::t!("sftp.file_ops.local_symlink_unsupported").to_string());
            match result {
                Ok(()) => LocalFsOutcome {
                    status: Some(
                        rust_i18n::t!("sftp.file_ops.local_symlink_created", name = link.display())
                            .to_string(),
                    ),
                    error: None,
                    select: Some(link),
                    reveal: None,
                },
                Err(error) => LocalFsOutcome {
                    status: None,
                    error: Some(
                        rust_i18n::t!("sftp.file_ops.local_symlink_failed", error = error)
                            .to_string(),
                    ),
                    select: None,
                    reveal: None,
                },
            }
        }
    }
}

impl AppShell {
    /// 在后台线程执行本地文件操作，完成后回 UI 刷新对应面板。
    fn spawn_local_fs(&mut self, tab_id: TabId, op: LocalFsOp, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Some(outcome) = spawn_blocking_local(move || run_local_fs(op)).await else {
                log::error!("本地文件操作线程创建失败");
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.apply_local_fs_outcome(tab_id, outcome, cx);
            });
        })
        .detach();
    }

    /// 应用本地文件操作结果：错误与状态写回面板，成功时刷新并选中目标。
    fn apply_local_fs_outcome(
        &mut self,
        tab_id: TabId,
        outcome: LocalFsOutcome,
        cx: &mut Context<Self>,
    ) {
        let (select, reveal) = {
            let Some(mut panel) = self.sftp_view_mut(tab_id, cx) else {
                return;
            };
            if let Some(error) = outcome.error {
                panel.state.set_local_error(error);
            }
            if let Some(status) = outcome.status {
                panel.state.set_status(status);
            }
            (outcome.select, outcome.reveal)
        };
        if let Some(path) = select {
            self.spawn_local_dir_refresh(tab_id, Some(path), cx);
        }
        if let Some(path) = reveal {
            crate::audit::reveal_in_file_manager(path);
        }
        cx.notify();
    }
}
