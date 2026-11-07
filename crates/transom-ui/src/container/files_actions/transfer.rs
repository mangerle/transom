//! 容器文件的上传与下载。

use super::super::*;

impl AppShell {
    /// 下载容器或数据卷内文件。
    pub(crate) fn download_container_file(&mut self, tab_id: TabId, path: String) {
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(path.rsplit('/').next().unwrap_or("download"))
            .save_file()
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let is_volume = workspace.section == ContainerSection::Volumes;
        let target = if is_volume {
            workspace.selected_volume_name()
        } else {
            workspace.selected_container_name()
        };
        let Some(target) = target else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        let command = if is_volume {
            ContainerCommand::DownloadVolumeFile {
                tab_id,
                request_id,
                volume: target,
                path,
                local_path,
            }
        } else {
            ContainerCommand::Download {
                tab_id,
                request_id,
                container: target,
                path,
                local_path,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 把容器内目录导出为本地 tar 归档。
    pub(crate) fn download_container_directory(&mut self, tab_id: TabId, path: String) {
        let is_volume = self
            .state
            .container_workspace(tab_id)
            .is_some_and(|workspace| workspace.section == ContainerSection::Volumes);
        if is_volume {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.notice =
                    Some(rust_i18n::t!("container.files.directory_volume_unsupported").to_string());
            }
            return;
        }
        let name = path.rsplit('/').next().unwrap_or("directory");
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.tar"))
            .save_file()
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(target) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::DownloadDirectory {
                tab_id,
                request_id,
                container: target,
                path,
                local_path,
            },
        );
    }

    /// 上传指定本地路径的文件到容器当前目录；目标已存在时先确认覆盖。
    pub(crate) fn upload_container_path(&mut self, tab_id: TabId, local_path: std::path::PathBuf) {
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let directory = if workspace.files.path.is_empty() {
            "/".to_string()
        } else {
            workspace.files.path.clone()
        };
        let target = if directory == "/" {
            format!("/{file_name}")
        } else {
            format!("{directory}/{file_name}")
        };
        let is_dir = local_path.is_dir();
        if workspace
            .files
            .entries
            .iter()
            .any(|entry| entry.name == file_name)
        {
            workspace.confirm = Some(ContainerConfirm::UploadOverwrite {
                path: target,
                local_path,
                directory: is_dir,
            });
            return;
        }
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        let command = if is_dir {
            ContainerCommand::UploadDirectory {
                tab_id,
                request_id,
                container,
                path: target,
                local_path,
            }
        } else {
            ContainerCommand::Upload {
                tab_id,
                request_id,
                container,
                path: target,
                local_path,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 上传本地文件到当前目录。
    pub(crate) fn upload_container_file(&mut self, tab_id: TabId) {
        let Some(local_path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        self.upload_container_path(tab_id, local_path);
    }

    /// 上传本地目录到当前目录，目录层级原样保留。
    pub(crate) fn upload_container_directory(&mut self, tab_id: TabId) {
        let Some(local_path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        self.upload_container_path(tab_id, local_path);
    }
}
