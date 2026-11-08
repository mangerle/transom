//! 容器文件页的目录加载与预览。

use super::super::*;

impl ContainerWorkspace {
    /// 首次进入文件页：沿用上次目录，空路径按根目录处理。
    pub(crate) fn ensure_container_files(&mut self, cx: &mut Context<Self>) {
        let path = if self.state.files.path.is_empty() {
            "/".to_string()
        } else {
            self.state.files.path.clone()
        };
        self.load_container_dir(path, cx);
    }

    /// 加载容器或数据卷内的目录。
    ///
    /// 目录切换可能连点：先登记新请求再取消旧请求，旧结果因登记被取走而丢弃。
    pub(crate) fn load_container_dir(&mut self, path: String, cx: &mut Context<Self>) {
        let Some((is_volume, target)) = self.file_target() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.state.files.loading = true;
        self.state.files.preview = None;
        let previous = self.state.files.request.replace(request_id);
        if let Some(previous) = previous {
            self.cancel_container_request(previous);
        }
        let command = if is_volume {
            log::info!("打开数据卷目录，卷: {target}，路径: {path}，请求: {request_id}");
            ContainerCommand::ListVolumeDir {
                tab_id: self.tab_id,
                request_id,
                volume: target,
                path,
            }
        } else {
            log::info!("打开容器目录，容器: {target}，路径: {path}，请求: {request_id}");
            ContainerCommand::ListDir {
                tab_id: self.tab_id,
                request_id,
                container: target,
                path,
            }
        };
        self.send_tracked(request_id, command);
        cx.notify();
    }

    /// 预览容器或数据卷内文本文件。
    pub(crate) fn preview_container_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some((is_volume, target)) = self.file_target() else {
            return;
        };
        self.state.files.preview_editor = None;
        self.state.files.preview_saving = false;
        self.state.files.preview_binary = false;
        let request_id = ContainerRequestId::new();
        let command = if is_volume {
            ContainerCommand::ReadVolumeFile {
                tab_id: self.tab_id,
                request_id,
                volume: target,
                path,
                max_bytes: 1024 * 1024,
            }
        } else {
            ContainerCommand::ReadFile {
                tab_id: self.tab_id,
                request_id,
                container: target,
                path,
                max_bytes: 1024 * 1024,
            }
        };
        self.send_tracked(request_id, command);
        cx.notify();
    }
}
