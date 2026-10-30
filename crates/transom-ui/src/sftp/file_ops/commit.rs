//! SFTP 模态输入框提交处理与本地软链接创建。

use super::validate::name_validation_error;
use crate::app::AppShell;
use crate::sftp::{SftpInputTarget, SftpRemotePaneRef, join_remote, parent_remote};
use gpui_kit::*;
use std::path::Path;
use transom_core::{SftpOperation, TabId};

impl AppShell {
    /// 提交当前打开的 SFTP 输入模态对话框，执行创建文件夹、新建文件编辑、重命名或软链接。
    pub(crate) fn commit_sftp_input_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_modals.input.take() else {
            return;
        };
        let tab_id = modal.tab_id;
        let name = modal.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            let mut modal = modal;
            modal.error = Some(rust_i18n::t!("sftp.file_ops.name_empty").to_string());
            let input = modal.input.clone();
            self.sftp_modals.input = Some(modal);
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if let Some(message) = name_validation_error(&modal.target, &name) {
            let mut modal = modal;
            modal.error = Some(message);
            let input = modal.input.clone();
            self.sftp_modals.input = Some(modal);
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
        let remote_scope = self.remote_pane_scope(tab_id, target).map(|scope| {
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
                    self.sftp_modals.input = Some(modal);
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
                if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
                    let new_dir = panel.local.path.join(&name);
                    if let Err(err) = std::fs::create_dir_all(&new_dir) {
                        panel.set_local_error(
                            rust_i18n::t!("sftp.file_ops.local_mkdir_failed", error = err)
                                .to_string(),
                        );
                    } else {
                        panel.set_status(
                            rust_i18n::t!(
                                "sftp.file_ops.local_mkdir_created",
                                name = name.as_str()
                            )
                            .to_string(),
                        );
                        self.spawn_local_dir_refresh(tab_id, None, cx);
                    }
                    cx.notify();
                }
            }
            SftpInputTarget::LocalNewFile => {
                if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
                    let new_file = panel.local.path.join(&name);
                    if new_file.exists() {
                        panel.set_local_error(
                            rust_i18n::t!("sftp.file_ops.local_file_exists", name = name.as_str())
                                .to_string(),
                        );
                    } else {
                        match std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&new_file)
                        {
                            Ok(_) => {
                                panel.set_status(
                                    rust_i18n::t!(
                                        "sftp.file_ops.local_file_created",
                                        name = name.as_str()
                                    )
                                    .to_string(),
                                );
                                self.spawn_local_dir_refresh(tab_id, Some(new_file.clone()), cx);
                                crate::audit::reveal_in_file_manager(new_file);
                            }
                            Err(err) => {
                                panel.set_local_error(
                                    rust_i18n::t!(
                                        "sftp.file_ops.local_file_create_failed",
                                        error = err
                                    )
                                    .to_string(),
                                );
                            }
                        }
                    }
                    cx.notify();
                }
            }
            SftpInputTarget::LocalRename { old_path } => {
                if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
                    if let Some(parent) = old_path.parent() {
                        let new_path = parent.join(&name);
                        if let Err(err) = std::fs::rename(&old_path, &new_path) {
                            panel.set_local_error(
                                rust_i18n::t!("sftp.file_ops.local_rename_failed", error = err)
                                    .to_string(),
                            );
                        } else {
                            panel.set_status(
                                rust_i18n::t!("sftp.file_ops.local_renamed", name = name.as_str())
                                    .to_string(),
                            );
                            self.spawn_local_dir_refresh(tab_id, Some(new_path), cx);
                        }
                    }
                    cx.notify();
                }
            }
            SftpInputTarget::LocalSymlink { target_path } => {
                self.create_local_symlink(tab_id, &target_path, &name, cx);
            }
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }

    /// 在当前本地目录创建指向指定目标的软链接（仅 Unix 平台支持）。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：本地栏与远端栏保持右键菜单能力对称，避免用户在两栏间形成心智差异；
    /// - 核心优势：直接复用标准库 `std::os::unix::fs::symlink`，无额外依赖，
    ///   且创建后立即刷新并选中新链接，反馈闭环完整；
    /// - 代价与局限：Windows 需要特权与文件/目录区分调用，当前不提供该平台支持，
    ///   以显式报错提示代替静默失败。
    fn create_local_symlink(
        &mut self,
        tab_id: TabId,
        target_path: &Path,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) else {
            return;
        };
        let link_path = panel.local.path.join(name);
        #[cfg(unix)]
        let result =
            std::os::unix::fs::symlink(target_path, &link_path).map_err(|error| error.to_string());
        #[cfg(not(unix))]
        let result = Err(rust_i18n::t!("sftp.file_ops.local_symlink_unsupported").to_string());
        match result {
            Ok(()) => {
                panel.set_status(
                    rust_i18n::t!("sftp.file_ops.local_symlink_created", name = name).to_string(),
                );
                self.spawn_local_dir_refresh(tab_id, Some(link_path), cx);
            }
            Err(err) => {
                panel.set_local_error(
                    rust_i18n::t!("sftp.file_ops.local_symlink_failed", error = err).to_string(),
                );
            }
        }
        cx.notify();
    }
}
