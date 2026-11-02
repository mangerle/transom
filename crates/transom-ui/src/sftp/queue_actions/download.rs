//! 远端列下载：把目标列选中条目加入本地下载队列。

use crate::app::app_settings;
use crate::palette::localized_error;
use crate::sftp::entity::SftpPanelView;
use crate::sftp::{SftpRemotePaneRef, TransferTask};
use gpui_kit::*;
use transom_core::{TransferDirection, VerifyOptions};

impl SftpPanelView {
    /// 下载目标远端列当前选中的条目到本地当前目录。
    pub(crate) fn download_pane(&mut self, target: SftpRemotePaneRef, cx: &mut Context<Self>) {
        let Some(main_connection) = self.state.main_connection else {
            return;
        };
        let verify = VerifyOptions {
            enabled: app_settings(cx).sftp.verify_after_transfer,
            mode: app_settings(cx).sftp.verify_mode,
            algorithm: app_settings(cx).sftp.checksum_algorithm,
        };
        let Some(source_connection_id) =
            self.state.remote_connection(target, Some(main_connection))
        else {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error =
                    Some(rust_i18n::t!("sftp.queue_actions.no_remote_download").to_string());
            }
            cx.notify();
            return;
        };
        let paths = self.state.selected_remote_paths_pane(target);
        if paths.is_empty() {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        }
        let tasks = match build_download_tasks(DownloadTasksInput {
            paths,
            source_connection_id,
            verify,
            local_dir: &self.state.local.path,
            entries: self
                .state
                .remote_pane(target)
                .map(|pane| pane.entries.as_slice())
                .unwrap_or_default(),
        }) {
            Ok(tasks) => tasks,
            Err(message) => {
                if let Some(pane) = self.state.remote_pane_mut(target) {
                    pane.error = Some(message);
                }
                cx.notify();
                return;
            }
        };
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

/// 下载任务构建输入：选中远端路径、目标目录与校验策略。
struct DownloadTasksInput<'a> {
    paths: Vec<String>,
    entries: &'a [transom_core::RemoteFile],
    local_dir: &'a std::path::Path,
    source_connection_id: transom_core::ConnectionId,
    verify: VerifyOptions,
}

/// 把选中远端路径转成下载任务；非法文件名直接返回错误文案。
fn build_download_tasks(input: DownloadTasksInput<'_>) -> Result<Vec<TransferTask>, String> {
    let DownloadTasksInput {
        paths,
        entries,
        local_dir,
        source_connection_id,
        verify,
    } = input;
    let mut tasks = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(remote_entry) = entries.iter().find(|entry| entry.path == path) else {
            continue;
        };
        let local_path = match transom_core::safe_local_join(local_dir, &remote_entry.name) {
            Ok(path) => path,
            Err(error) => return Err(localized_error(&error)),
        };
        let mut task = if remote_entry.is_dir {
            TransferTask::new_directory(
                TransferDirection::Download,
                local_path,
                remote_entry.path.clone(),
                None,
            )
        } else {
            TransferTask::new(
                TransferDirection::Download,
                local_path,
                remote_entry.path.clone(),
                Some(remote_entry.size),
            )
        };
        task.source_connection_id = Some(source_connection_id);
        task.verify = verify;
        tasks.push(task);
    }
    Ok(tasks)
}
