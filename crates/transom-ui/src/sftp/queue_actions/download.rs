//! 远端列下载：把目标列选中条目加入本地下载队列。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::sftp::{SftpRemotePaneRef, TransferMode, TransferTask};
use gpui_kit::*;
use transom_core::{TabId, TransferDirection};

impl AppShell {
    /// 下载目标远端列当前选中的条目到本地当前目录。
    ///
    /// 主远端与并列远端共用同一条构建路径，只在"源连接"上区分；证书、
    /// 目录类型与本地路径校验不允许两套实现，否则跨列下载的口径会漂移。
    pub(crate) fn sftp_queue_download_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let verify = self.settings.sftp.verify_after_transfer;
        let verify_mode = self.settings.sftp.verify_mode;
        let checksum_algorithm = self.settings.sftp.checksum_algorithm;
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let main_conn = tab.connection_id;
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        let Some(source_connection_id) = panel.remote_connection(target, main_conn) else {
            if let Some(pane) = panel.remote_pane_mut(target) {
                pane.error =
                    Some(rust_i18n::t!("sftp.queue_actions.no_remote_download").to_string());
            }
            cx.notify();
            return;
        };
        let mode = if panel.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let paths = panel.selected_remote_paths_pane(target);
        if paths.is_empty() {
            if let Some(pane) = panel.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        }
        let (tasks, invalid_name) = {
            let Some(pane) = panel.remote_pane(target) else {
                return;
            };
            let mut tasks = Vec::with_capacity(paths.len());
            let mut invalid_name: Option<String> = None;
            for path in paths {
                let Some(remote_entry) = pane.entries.iter().find(|entry| entry.path == path)
                else {
                    continue;
                };
                let local_path =
                    match transom_core::safe_local_join(&panel.local.path, &remote_entry.name) {
                        Ok(path) => path,
                        Err(error) => {
                            invalid_name = Some(localized_error(&error));
                            break;
                        }
                    };
                let mut task = if remote_entry.is_dir {
                    TransferTask::new_directory(
                        TransferDirection::Download,
                        local_path,
                        remote_entry.path.clone(),
                        None,
                        mode,
                    )
                } else {
                    TransferTask::new(
                        TransferDirection::Download,
                        local_path,
                        remote_entry.path.clone(),
                        Some(remote_entry.size),
                        mode,
                    )
                };
                task.source_connection_id = Some(source_connection_id);
                task.verify = verify;
                task.verify_mode = verify_mode;
                task.checksum_algorithm = checksum_algorithm;
                tasks.push(task);
            }
            (tasks, invalid_name)
        };
        if let Some(message) = invalid_name {
            if let Some(pane) = panel.remote_pane_mut(target) {
                pane.error = Some(message);
            }
            cx.notify();
            return;
        }
        if tasks.is_empty() {
            if let Some(pane) = panel.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        }
        let count = tasks.len();
        panel.queue.tasks.extend(tasks);
        panel.set_status(
            rust_i18n::t!("sftp.queue_actions.downloads_queued", count = count).to_string(),
        );
        cx.notify();
        self.process_sftp_queue(tab_id, cx);
    }
}
