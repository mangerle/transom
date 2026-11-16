//! 容器日志采样：拉取、跟随、行数与时间戳切换。

use super::*;
use crate::app::spawn_blocking_local;

impl ContainerWorkspace {
    /// 确保当前选中项的日志流已经建立；已有在途请求时不重复发起。
    pub(crate) fn ensure_container_logs(&mut self) {
        if self.state.logs_request.is_some() {
            return;
        }
        let follow = self.state.logs_follow;
        let tail = self.state.logs_tail;
        let timestamps = self.state.logs_timestamps;
        let (project, container) = if self.state.section == ContainerSection::Compose {
            let Some(project) = self.state.selected_compose() else {
                return;
            };
            let project_name = project.name.clone();
            let members: Vec<String> = self
                .state
                .containers
                .iter()
                .filter(|container| {
                    container.compose_project.as_deref() == Some(project_name.as_str())
                })
                .map(|container| container.name.clone())
                .collect();
            (Some((project_name, members)), None)
        } else {
            let Some(container) = self.state.selected_container_name() else {
                return;
            };
            (None, Some(container))
        };
        let request_id = ContainerRequestId::new();
        self.state.logs.clear();
        self.state.logs_request = Some(request_id);
        if let Some((project, containers)) = project {
            self.send_tracked(
                request_id,
                ContainerCommand::ProjectLogs {
                    tab_id: self.tab_id,
                    request_id,
                    project,
                    containers,
                    tail,
                    follow,
                    timestamps,
                },
            );
        } else if let Some(container) = container {
            self.send_tracked(
                request_id,
                ContainerCommand::Logs {
                    tab_id: self.tab_id,
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
    fn restart_container_logs(&mut self) {
        if let Some(request_id) = self.state.logs_request.take() {
            self.cancel_container_request(request_id);
        }
        self.ensure_container_logs();
    }

    /// 切换日志尾部行数档位（0 表示全量）。
    pub(crate) fn set_container_logs_tail(&mut self, tail: usize) {
        if self.state.logs_tail == tail {
            return;
        }
        self.state.logs_tail = tail;
        self.restart_container_logs();
    }

    /// 开关日志时间戳：由引擎附带时间戳，界面不做二次格式化。
    pub(crate) fn toggle_container_logs_timestamps(&mut self) {
        self.state.logs_timestamps = !self.state.logs_timestamps;
        self.restart_container_logs();
    }

    /// 把当前日志缓冲整体复制到剪贴板，便于贴到工单或聊天里。
    pub(crate) fn copy_container_logs(&mut self, cx: &mut Context<Self>) {
        let text = self.state.logs.clone();
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.notify();
    }

    /// 开关日志跟随：跟随状态变化后必须由引擎重开流才会生效。
    pub(crate) fn toggle_container_logs_follow(&mut self, follow: bool) {
        self.state.logs_follow = follow;
        self.restart_container_logs();
    }

    /// 将当前日志缓冲导出为本地 .log 文件。
    ///
    /// 日志可能上百 KB，写盘不能在渲染/点击线程同步做：交给阻塞线程池，
    /// 完成后再经工作区 `notice` 链路提示，避免大文件落盘卡住下一帧。
    pub(crate) fn export_container_logs(&mut self, cx: &mut Context<Self>) {
        let name = if self.state.section == ContainerSection::Compose {
            self.state
                .selected_compose()
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "compose".to_string())
        } else {
            self.state
                .selected_container_name()
                .unwrap_or_else(|| "container".to_string())
        };
        let text = self.state.logs.clone();
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

        let write_path = local_path.clone();
        cx.spawn(async move |this, cx| {
            let outcome = spawn_blocking_local(move || {
                std::fs::write(&write_path, text.as_bytes()).map_err(|error| error.to_string())
            })
            .await;
            let _ = this.update(cx, |workspace, cx| {
                let notice = match outcome {
                    Some(Ok(())) => rust_i18n::t!(
                        "container.logs.export_success",
                        path = local_path.display().to_string()
                    )
                    .to_string(),
                    Some(Err(error)) => {
                        log::error!(
                            "写入容器日志文件失败，路径: {}，原因: {error}",
                            local_path.display()
                        );
                        rust_i18n::t!("container.logs.export_failed", error = error).to_string()
                    }
                    None => {
                        log::error!(
                            "写入容器日志文件失败，路径: {}，原因: 后台写入线程不可用",
                            local_path.display()
                        );
                        rust_i18n::t!("container.logs.export_thread_failed").to_string()
                    }
                };
                workspace.state.notice = Some(notice);
                cx.notify();
            });
        })
        .detach();
    }
}
