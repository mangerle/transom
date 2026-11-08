//! 容器文件页的目录加载与预览。

use super::super::*;

impl AppShell {
    pub(crate) fn ensure_container_files(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let path = self
            .with_container(tab_id, cx, |workspace| workspace.files.path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/".to_string());
        self.load_container_dir(tab_id, path, cx);
    }

    pub(crate) fn load_container_dir(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
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
        let previous = self
            .with_container_mut(tab_id, cx, |workspace| {
                workspace.files.loading = true;
                workspace.files.preview = None;
                workspace.files.request.replace(request_id)
            })
            .flatten();
        if let Some(previous) = previous
            && let Some(workspace) = self.state.container_workspace(tab_id)
        {
            workspace.update(cx, |workspace, _| {
                workspace.cancel_container_request(previous)
            });
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
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(request_id, command);
            });
        }
    }

    /// 预览容器或数据卷内文本文件。
    pub(crate) fn preview_container_file(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
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
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.preview_editor = None;
            workspace.files.preview_saving = false;
            workspace.files.preview_binary = false;
        });
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
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(request_id, command);
            });
        }
    }
}
