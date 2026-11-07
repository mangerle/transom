//! 容器文件页的目录加载与预览。

use super::super::*;

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
        workspace.files.preview_editor = None;
        workspace.files.preview_saving = false;
        workspace.files.preview_binary = false;
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
}
