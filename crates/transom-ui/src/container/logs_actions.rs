//! 容器日志采样：拉取、跟随、行数与时间戳切换。

use super::*;
use crate::sftp::spawn_blocking_local;

impl AppShell {
    pub(crate) fn ensure_container_logs(&mut self, tab_id: TabId) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.logs_request.is_some() {
            return;
        }
        let follow = workspace.logs_follow;
        let tail = workspace.logs_tail;
        let timestamps = workspace.logs_timestamps;
        workspace.logs.clear();
        let request_id = ContainerRequestId::new();
        workspace.logs_request = Some(request_id);

        if workspace.section == ContainerSection::Compose {
            let Some(project) = workspace.selected_compose() else {
                return;
            };
            let project_name = project.name.clone();
            let member_containers: Vec<String> = workspace
                .containers
                .iter()
                .filter(|c| c.compose_project.as_deref() == Some(&project_name))
                .map(|c| c.name.clone())
                .collect();
            self.send_tracked(
                tab_id,
                request_id,
                ContainerCommand::ProjectLogs {
                    tab_id,
                    request_id,
                    project: project_name,
                    containers: member_containers,
                    tail,
                    follow,
                    timestamps,
                },
            );
        } else {
            let Some(container) = workspace.selected_container_name() else {
                return;
            };
            self.send_tracked(
                tab_id,
                request_id,
                ContainerCommand::Logs {
                    tab_id,
                    request_id,
                    container,
                    tail,
                    follow,
                    timestamps,
                },
            );
        }
    }

    /// 重开日志流：跟随、尾部行数与时间戳都要改才能生效，先取消旧流再拉新流。
    fn restart_container_logs(&mut self, tab_id: TabId) {
        if let Some(request_id) = self
            .state
            .container_workspace_mut(tab_id)
            .and_then(|workspace| workspace.logs_request.take())
        {
            self.cancel_container_request(tab_id, request_id);
        }
        self.ensure_container_logs(tab_id);
    }

    /// 切换日志尾部行数档位（0 表示全量）。
    pub(crate) fn set_container_logs_tail(&mut self, tab_id: TabId, tail: usize) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if workspace.logs_tail == tail {
                return;
            }
            workspace.logs_tail = tail;
        }
        self.restart_container_logs(tab_id);
    }

    /// 开关日志时间戳：由引擎附带时间戳，界面不做二次格式化。
    pub(crate) fn toggle_container_logs_timestamps(&mut self, tab_id: TabId) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.logs_timestamps = !workspace.logs_timestamps;
        }
        self.restart_container_logs(tab_id);
    }

    /// 把当前日志缓冲整体复制到剪贴板，便于贴到工单或聊天里。
    pub(crate) fn copy_container_logs(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(text) = self
            .state
            .container_workspace(tab_id)
            .map(|workspace| workspace.logs.clone())
        else {
            return;
        };
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.notify();
    }

    pub(crate) fn toggle_container_logs_follow(&mut self, tab_id: TabId, follow: bool) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.logs_follow = follow;
        }
        self.restart_container_logs(tab_id);
    }

    /// 将当前日志缓冲导出为本地 .log 文件。
    pub(crate) fn export_container_logs(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let (name, text) = match self.state.container_workspace(tab_id) {
            Some(workspace) => {
                let name = if workspace.section == ContainerSection::Compose {
                    workspace
                        .selected_compose()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "compose".to_string())
                } else {
                    workspace
                        .selected_container_name()
                        .unwrap_or_else(|| "container".to_string())
                };
                (name, workspace.logs.clone())
            }
            None => return,
        };
        if text.is_empty() {
            return;
        }

        let default_name = format!("{name}.log");
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Log", &["log", "txt"])
            .save_file()
        else {
            return;
        };

        // 日志可能上百 KB，写盘不能在渲染/点击线程同步做：交给阻塞线程池，
        // 完成后再回到 UI 线程弹提示，避免大文件落盘卡住下一帧。
        let write_path = local_path.clone();
        cx.spawn(async move |this, cx| {
            let outcome = spawn_blocking_local(move || {
                std::fs::write(&write_path, text.as_bytes()).map_err(|error| error.to_string())
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                Some(Ok(())) => {
                    let path_str = local_path.display().to_string();
                    this.show_toast(
                        rust_i18n::t!("container.logs.export_success", path = path_str),
                        true,
                        window,
                        cx,
                    );
                }
                Some(Err(error)) => {
                    log::error!(
                        "写入容器日志文件失败，路径: {}，原因: {error}",
                        local_path.display()
                    );
                    this.show_toast(
                        rust_i18n::t!("container.logs.export_failed", error = error),
                        false,
                        window,
                        cx,
                    );
                }
                None => {
                    log::error!(
                        "写入容器日志文件失败，路径: {}，原因: 后台写入线程不可用",
                        local_path.display()
                    );
                    this.show_toast(
                        rust_i18n::t!("container.logs.export_thread_failed").to_string(),
                        false,
                        window,
                        cx,
                    );
                }
            });
        })
        .detach();
    }
}
