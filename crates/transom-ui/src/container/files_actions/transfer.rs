//! 容器文件的上传与下载。

use super::super::*;

impl AppShell {
    /// 下载容器或数据卷内文件。
    pub(crate) fn download_container_file(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(path.rsplit('/').next().unwrap_or("download"))
            .save_file()
        else {
            return;
        };
        let Some((is_volume, target)) = self
            .with_container(tab_id, cx, |workspace| {
                let is_volume = workspace.section == ContainerSection::Volumes;
                let target = if is_volume {
                    workspace.selected_volume_name()
                } else {
                    workspace.selected_container_name()
                }?;
                Some((is_volume, target))
            })
            .flatten()
        else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        });
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
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(request_id, command);
            });
        }
    }

    /// 把容器内目录导出为本地 tar 归档。
    pub(crate) fn download_container_directory(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let is_volume = self
            .with_container(tab_id, cx, |workspace| {
                workspace.section == ContainerSection::Volumes
            })
            .unwrap_or(false);
        if is_volume {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.notice =
                    Some(rust_i18n::t!("container.files.directory_volume_unsupported").to_string());
            });
            return;
        }
        let name = path.rsplit('/').next().unwrap_or("directory");
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.tar"))
            .save_file()
        else {
            return;
        };
        let Some(target) = self
            .with_container(tab_id, cx, |workspace| workspace.selected_container_name())
            .flatten()
        else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        });
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::DownloadDirectory {
                        tab_id,
                        request_id,
                        container: target,
                        path,
                        local_path,
                    },
                );
            });
        }
    }

    /// 上传指定本地路径的文件到容器当前目录；目标已存在时先确认覆盖。
    pub(crate) fn upload_container_path(
        &mut self,
        tab_id: TabId,
        local_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return;
        };
        let Some((directory, overwrite, container)) = self
            .with_container(tab_id, cx, |workspace| {
                let directory = if workspace.files.path.is_empty() {
                    "/".to_string()
                } else {
                    workspace.files.path.clone()
                };
                let overwrite = workspace
                    .files
                    .entries
                    .iter()
                    .any(|entry| entry.name == file_name);
                let container = workspace.selected_container_name()?;
                Some((directory, overwrite, container))
            })
            .flatten()
        else {
            return;
        };
        let target = if directory == "/" {
            format!("/{file_name}")
        } else {
            format!("{directory}/{file_name}")
        };
        let is_dir = local_path.is_dir();
        if overwrite {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.confirm = Some(ContainerConfirm::UploadOverwrite {
                    path: target,
                    local_path,
                    directory: is_dir,
                });
            });
            return;
        }
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        });
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
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(request_id, command);
            });
        }
    }

    /// 上传本地文件到当前目录。
    pub(crate) fn upload_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(local_path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        self.upload_container_path(tab_id, local_path, cx);
    }

    /// 上传本地目录到当前目录，目录层级原样保留。
    pub(crate) fn upload_container_directory(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(local_path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        self.upload_container_path(tab_id, local_path, cx);
    }
}
