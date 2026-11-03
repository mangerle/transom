//! 容器文件页的浏览与传输动作。

use super::*;

impl AppShell {
    pub(crate) fn ensure_container_files(&mut self, tab_id: TabId) {
        let path = self
            .state
            .container_workspace_mut(tab_id)
            .map(|workspace| workspace.files.path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/".to_string());
        self.load_container_dir(tab_id, path);
    }

    pub(crate) fn load_container_dir(&mut self, tab_id: TabId, path: String) {
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
        workspace.files.loading = true;
        workspace.files.preview = None;
        let request_id = ContainerRequestId::new();
        if let Some(previous) = workspace.files.request.replace(request_id) {
            self.cancel_container_request(tab_id, previous);
        }
        let command = if is_volume {
            log::info!("打开数据卷目录，卷: {target}，路径: {path}，请求: {request_id}");
            ContainerCommand::ListVolumeDir {
                tab_id,
                request_id,
                volume: target,
                path,
            }
        } else {
            log::info!("打开容器目录，容器: {target}，路径: {path}，请求: {request_id}");
            ContainerCommand::ListDir {
                tab_id,
                request_id,
                container: target,
                path,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 预览容器或数据卷内文本文件。
    pub(crate) fn preview_container_file(&mut self, tab_id: TabId, path: String) {
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
        let command = if is_volume {
            ContainerCommand::ReadVolumeFile {
                tab_id,
                request_id,
                volume: target,
                path,
                max_bytes: 1024 * 1024,
            }
        } else {
            ContainerCommand::ReadFile {
                tab_id,
                request_id,
                container: target,
                path,
                max_bytes: 1024 * 1024,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

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

    /// 打开文件重命名条：按钮与右键菜单共用同一份输入状态。
    pub(crate) fn begin_container_file_rename(
        &mut self,
        tab_id: TabId,
        path: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.files.rename_from = Some(path);
            workspace.files.rename_input = Some(input);
        }
        cx.notify();
    }

    /// 请求删除容器内路径；目录按递归删除走二次确认。
    pub(crate) fn request_remove_container_path(
        &mut self,
        tab_id: TabId,
        path: String,
        recursive: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.confirm = Some(ContainerConfirm::FilePath { path, recursive });
        }
        cx.notify();
    }

    /// 上传本地文件到当前目录。
    pub(crate) fn upload_container_file(&mut self, tab_id: TabId) {
        let Some(local_path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
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
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Upload {
                tab_id,
                request_id,
                container,
                path: target,
                local_path,
            },
        );
    }

    /// 重命名容器内路径。
    /// 展开或收起「新建目录」条；展开时才创建输入框，避免常驻实体。
    pub(crate) fn toggle_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.new_dir_open = !workspace.files.new_dir_open;
        if workspace.files.new_dir_open && workspace.files.new_dir_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("container.files.new_dir_placeholder").to_string())
            });
            workspace.files.new_dir_input = Some(input);
        }
        cx.notify();
    }

    /// 在容器当前目录里新建子目录：上传与整理文件都需要先有目录。
    pub(crate) fn submit_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let Some(input) = workspace.files.new_dir_input.clone() else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() || name.contains('/') || name.contains("..") {
            self.show_toast(
                rust_i18n::t!("container.files.new_dir_invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        }
        let parent = if workspace.files.path.is_empty() {
            "/".to_string()
        } else {
            workspace.files.path.clone()
        };
        let path = if parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}/{name}")
        };
        let request_id = ContainerRequestId::new();
        workspace.files.new_dir_open = false;
        workspace.busy = true;
        workspace
            .files
            .new_dir_input
            .as_ref()
            .iter()
            .for_each(|input| input.update(cx, |input, cx| input.set_value("", window, cx)));
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::CreateDir {
                tab_id,
                request_id,
                container,
                path,
            },
        );
        cx.notify();
    }

    pub(crate) fn rename_container_path(&mut self, tab_id: TabId, from: String, to: String) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.files.rename_from = None;
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::Rename {
                tab_id,
                request_id,
                container,
                from,
                to,
            },
        );
    }
}
