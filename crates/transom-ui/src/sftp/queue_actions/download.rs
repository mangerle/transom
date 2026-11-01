//! 远端列下载：把目标列选中条目加入本地下载队列。

use crate::app::app_settings;
use crate::palette::localized_error;
use crate::sftp::entity::{SftpPanelView, read_shell_parts};
use crate::sftp::{SftpRemotePaneRef, TransferMode, TransferTask};
use gpui_kit::*;
use transom_core::TransferDirection;

impl SftpPanelView {
    /// 下载目标远端列当前选中的条目到本地当前目录。
    pub(crate) fn download_pane(&mut self, target: SftpRemotePaneRef, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let Some((main_connection, verify, verify_mode, checksum_algorithm)) =
            read_shell_parts(&app, cx, |shell| {
                (
                    shell
                        .state
                        .workspace
                        .tab(self.tab_id)
                        .and_then(|tab| tab.connection_id),
                    app_settings(cx).sftp.verify_after_transfer,
                    app_settings(cx).sftp.verify_mode,
                    app_settings(cx).sftp.checksum_algorithm,
                )
            })
        else {
            return;
        };
        let Some(source_connection_id) = self.state.remote_connection(target, main_connection)
        else {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error =
                    Some(rust_i18n::t!("sftp.queue_actions.no_remote_download").to_string());
            }
            cx.notify();
            return;
        };
        let mode = if self.state.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let paths = self.state.selected_remote_paths_pane(target);
        if paths.is_empty() {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        }
        let mut tasks = Vec::with_capacity(paths.len());
        let mut invalid_name: Option<String> = None;
        {
            let Some(pane) = self.state.remote_pane(target) else {
                return;
            };
            for path in paths {
                let Some(remote_entry) = pane.entries.iter().find(|entry| entry.path == path)
                else {
                    continue;
                };
                let local_path =
                    match transom_core::safe_local_join(&self.state.local.path, &remote_entry.name)
                    {
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
        }
        if let Some(message) = invalid_name {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(message);
            }
            cx.notify();
            return;
        }
        if tasks.is_empty() {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        }
        let count = tasks.len();
        self.state.queue.tasks.extend(tasks);
        self.state.set_status(
            rust_i18n::t!("sftp.queue_actions.downloads_queued", count = count).to_string(),
        );
        self.process_queue(self.tab_id, cx);
        cx.notify();
    }
}
