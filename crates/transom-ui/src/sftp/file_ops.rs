use super::{
    SftpChmodModalState, SftpInputModalState, SftpInputTarget, join_remote, list_local_dir,
    parent_remote,
};
use crate::app::AppShell;
use crate::palette::sftp_text_input;
use gpui_kit::*;
use transom_core::{
    SftpOperation, TabId, TransferDirection, TransferMode, TransferTask, format_octal_permissions,
    parse_octal_permissions,
};

impl AppShell {
    pub(crate) fn sftp_delete(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let op = {
            let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
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
            let Some(entry) = panel.entries.iter().find(|entry| entry.path == selected) else {
                panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
                cx.notify();
                return;
            };
            SftpOperation::Delete {
                path: entry.path.clone(),
                is_dir: entry.is_dir,
            }
        };
        self.send_sftp_op(tab_id, op, cx);
    }

    pub(crate) fn sftp_open_mkdir_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, prompt, target) = if is_remote {
            (
                "新建远端文件夹".to_string(),
                "请输入新建远端文件夹的名称:".to_string(),
                SftpInputTarget::RemoteMkdir,
            )
        } else {
            (
                "新建本地文件夹".to_string(),
                "请输入新建本地文件夹的名称:".to_string(),
                SftpInputTarget::LocalMkdir,
            )
        };
        let input = sftp_text_input(window, cx, "", "文件夹名称");
        self.sftp_input_modal = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn sftp_open_rename_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_ref() else {
            return;
        };
        let (title, prompt, initial_value, target) = if is_remote {
            let Some(selected) = panel.selected.clone() else {
                return;
            };
            let initial = selected
                .trim_end_matches('/')
                .rsplit_once('/')
                .map(|(_, name)| name.to_string())
                .unwrap_or_else(|| selected.clone());
            (
                "重命名远端文件".to_string(),
                "请输入新的名称:".to_string(),
                initial,
                SftpInputTarget::RemoteRename { old_path: selected },
            )
        } else {
            let Some(selected) = panel.local_selected.clone() else {
                return;
            };
            let initial = selected
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (
                "重命名本地文件".to_string(),
                "请输入新的名称:".to_string(),
                initial,
                SftpInputTarget::LocalRename { old_path: selected },
            )
        };
        let input = sftp_text_input(window, cx, &initial_value, "新名称");
        self.sftp_input_modal = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn commit_sftp_input_modal(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_input_modal.take() else {
            return;
        };
        let tab_id = modal.tab_id;
        let name = modal.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            let mut modal = modal;
            modal.error = Some("名称不能为空".to_string());
            self.sftp_input_modal = Some(modal);
            cx.notify();
            return;
        }

        match modal.target {
            SftpInputTarget::RemoteMkdir => {
                let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
                    return;
                };
                let Some(panel) = tab.sftp.as_ref() else {
                    return;
                };
                let op = SftpOperation::Mkdir {
                    path: join_remote(&panel.path, &name),
                };
                self.send_sftp_op(tab_id, op, cx);
            }
            SftpInputTarget::RemoteRename { old_path } => {
                let op = SftpOperation::Rename {
                    from: old_path.clone(),
                    to: join_remote(&parent_remote(&old_path), &name),
                };
                self.send_sftp_op(tab_id, op, cx);
            }
            SftpInputTarget::LocalMkdir => {
                if let Some(panel) = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp.as_mut())
                {
                    let new_dir = panel.local_path.join(&name);
                    if let Err(err) = std::fs::create_dir_all(&new_dir) {
                        panel.local_error = Some(format!("新建本地文件夹失败: {err}"));
                    } else {
                        panel.local_entries = list_local_dir(&panel.local_path).unwrap_or_default();
                        panel.status = Some(format!("已成功创建本地文件夹: {name}"));
                    }
                    cx.notify();
                }
            }
            SftpInputTarget::LocalRename { old_path } => {
                if let Some(panel) = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp.as_mut())
                {
                    if let Some(parent) = old_path.parent() {
                        let new_path = parent.join(&name);
                        if let Err(err) = std::fs::rename(&old_path, &new_path) {
                            panel.local_error = Some(format!("重命名本地文件失败: {err}"));
                        } else {
                            panel.local_entries =
                                list_local_dir(&panel.local_path).unwrap_or_default();
                            panel.local_selected = Some(new_path);
                            panel.status = Some(format!("已重命名为: {name}"));
                        }
                    }
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn sftp_local_delete(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
        {
            let res = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match res {
                Ok(_) => {
                    if panel.local_selected.as_ref() == Some(&path) {
                        panel.local_selected = None;
                    }
                    panel.local_entries = list_local_dir(&panel.local_path).unwrap_or_default();
                    panel.status = Some("已成功删除本地项目".to_string());
                }
                Err(err) => {
                    panel.local_error = Some(format!("删除本地项目失败: {err}"));
                }
            }
            cx.notify();
        }
    }

    pub(crate) fn sftp_edit_remote_file(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        let Some(entry) = panel.entries.iter().find(|e| e.path == remote_path) else {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        };
        if entry.is_dir {
            return;
        }
        let edit_dir = std::env::temp_dir()
            .join("transom-sftp-edit")
            .join(tab_id.to_string());
        if let Err(err) = std::fs::create_dir_all(&edit_dir) {
            panel.error = Some(format!("创建临时编辑目录失败: {err}"));
            cx.notify();
            return;
        }
        let sanitized_name = entry.path.trim_start_matches('/').replace('/', "__");
        let local_path = edit_dir.join(sanitized_name);
        let task = TransferTask::new(
            TransferDirection::Download,
            local_path.clone(),
            entry.path.clone(),
            Some(entry.size),
            TransferMode::Overwrite,
        );
        panel.pending_edit_ops.insert(
            task.id,
            (local_path, entry.path.clone(), entry.name.clone()),
        );
        panel.queue.push(task);
        panel.status = Some(format!("正在拉取文件以供编辑: {}", entry.name));
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn save_sftp_editor_modal(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_editor_modal.take() else {
            return;
        };
        let content = modal.textarea.read(cx).value().to_string();
        if let Err(err) = std::fs::write(&modal.local_path, content.as_bytes()) {
            let mut modal = modal;
            modal.error = Some(format!("写入临时缓存文件失败: {err}"));
            self.sftp_editor_modal = Some(modal);
            cx.notify();
            return;
        }
        let tab_id = modal.tab_id;
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id)
            && let Some(panel) = tab.sftp.as_mut()
        {
            let task = TransferTask::new(
                TransferDirection::Upload,
                modal.local_path,
                modal.remote_path,
                Some(content.len() as u64),
                TransferMode::Overwrite,
            );
            panel.queue.push(task);
            panel.status = Some(format!("已保存并回传远端文件: {}", modal.file_name));
        }
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }

    pub(crate) fn sftp_open_chmod_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
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
        self.sftp_chmod_modal = Some(SftpChmodModalState {
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
        let Some(modal) = self.sftp_chmod_modal.as_mut() else {
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
        let Some(modal) = self.sftp_chmod_modal.as_mut() else {
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

    pub(crate) fn save_sftp_chmod_modal(&mut self, cx: &mut Context<Self>) {
        let Some(mut modal) = self.sftp_chmod_modal.take() else {
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
                    self.sftp_chmod_modal = Some(modal);
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
    }
}
