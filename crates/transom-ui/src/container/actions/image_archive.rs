//! 镜像归档导出与导入交互动作（Save / Load）。
//!
//! 设计原理（Why）：
//! - 镜像另存为 Tar 归档（Save）：通过系统文件保存对话框选取目标路径，流式从 Docker 引擎拉取镜像数据并写入本地磁盘；
//! - 镜像从本地 Tar 导入（Load）：选取本地 Tar/Tar.gz 归档并流式载入 Docker 引擎；
//! - 发送追踪命令并设置通知文案，让用户获得即时操作反馈。

use super::*;

impl AppShell {
    /// 导出指定镜像为本地 Tar 归档文件（Save）。
    pub(crate) fn save_image(&mut self, tab_id: TabId, id: String, cx: &mut Context<Self>) {
        let reference = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| {
                let state = &workspace.read(cx).state;
                state
                    .images
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.display_name())
            })
            .unwrap_or_else(|| id.clone());

        let default_name = format!("{}.tar", reference.replace(['/', ':'], "_"));
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Tar Archive", &["tar"])
            .save_file()
        else {
            return;
        };

        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.state.notice =
                    Some(rust_i18n::t!("container.transfer.start").to_string());
            });
        }
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::SaveImage {
                        tab_id,
                        request_id,
                        image: reference,
                        local_path,
                    },
                );
            });
        }
        cx.notify();
    }

    /// 从本地 Tar 归档文件导入镜像（Load）。
    pub(crate) fn load_image(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(local_path) = rfd::FileDialog::new()
            .add_filter("Docker Image", &["tar", "tar.gz", "tgz"])
            .pick_file()
        else {
            return;
        };

        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.state.notice =
                    Some(rust_i18n::t!("container.transfer.start").to_string());
            });
        }
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::LoadImage {
                        tab_id,
                        request_id,
                        local_path,
                    },
                );
            });
        }
        cx.notify();
    }
}
