//! 容器文件的上传与下载。

use super::super::*;

impl ContainerWorkspace {
    /// 下载容器或数据卷内文件。
    pub(crate) fn download_container_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(path.rsplit('/').next().unwrap_or("download"))
            .save_file()
        else {
            return;
        };
        let Some((is_volume, target)) = self.file_target() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.state.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        let command = if is_volume {
            ContainerCommand::DownloadVolumeFile {
                tab_id: self.tab_id,
                request_id,
                volume: target,
                path,
                local_path,
            }
        } else {
            ContainerCommand::Download {
                tab_id: self.tab_id,
                request_id,
                container: target,
                path,
                local_path,
            }
        };
        self.send_tracked(request_id, command);
        cx.notify();
    }

    /// 把容器内目录导出为本地 tar 归档。
    pub(crate) fn download_container_directory(&mut self, path: String, cx: &mut Context<Self>) {
        let is_volume = self.state.section == ContainerSection::Volumes;
        if is_volume {
            self.state.notice =
                Some(rust_i18n::t!("container.files.directory_volume_unsupported").to_string());
            cx.notify();
            return;
        }
        let name = path.rsplit('/').next().unwrap_or("directory");
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.tar"))
            .save_file()
        else {
            return;
        };
        let Some(target) = self.state.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.state.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        self.send_tracked(
            request_id,
            ContainerCommand::DownloadDirectory {
                tab_id: self.tab_id,
                request_id,
                container: target,
                path,
                local_path,
            },
        );
        cx.notify();
    }

    /// 上传指定本地路径的文件到容器当前目录；目标已存在时先确认覆盖。
    pub(crate) fn upload_container_path(
        &mut self,
        local_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return;
        };
        let directory = if self.state.files.path.is_empty() {
            "/".to_string()
        } else {
            self.state.files.path.clone()
        };
        let overwrite = self
            .state
            .files
            .entries
            .iter()
            .any(|entry| entry.name == file_name);
        let Some(container) = self.state.selected_container_name() else {
            return;
        };
        let target = if directory == "/" {
            format!("/{file_name}")
        } else {
            format!("{directory}/{file_name}")
        };
        let is_dir = local_path.is_dir();
        if overwrite {
            self.state.confirm = Some(ContainerConfirm::UploadOverwrite {
                path: target,
                local_path,
                directory: is_dir,
            });
            cx.notify();
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        let command = if is_dir {
            ContainerCommand::UploadDirectory {
                tab_id: self.tab_id,
                request_id,
                container,
                path: target,
                local_path,
            }
        } else {
            ContainerCommand::Upload {
                tab_id: self.tab_id,
                request_id,
                container,
                path: target,
                local_path,
            }
        };
        self.send_tracked(request_id, command);
        cx.notify();
    }

    /// 上传本地文件到当前目录。
    pub(crate) fn upload_container_file(&mut self, cx: &mut Context<Self>) {
        let Some(local_path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        self.upload_container_path(local_path, cx);
    }

    /// 上传本地目录到当前目录，目录层级原样保留。
    pub(crate) fn upload_container_directory(&mut self, cx: &mut Context<Self>) {
        let Some(local_path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        self.upload_container_path(local_path, cx);
    }
}
