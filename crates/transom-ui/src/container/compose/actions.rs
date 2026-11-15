//! Compose 项目的运行、创建、导入与打开动作。

use super::super::*;
use super::yaml::{compose_project_yaml, compose_yaml_is_allowed, unique_compose_destination};
use crate::app::spawn_blocking_local;
use chrono::Local;
use std::path::{Path, PathBuf};
use transom_core::{ComposeAction, ComposeProject};
use transom_platform::paths;

impl ContainerWorkspace {
    /// 编排项目命令：`up` 直接下发，`down` 会删容器与网络，先过确认条。
    ///
    /// 设计原理（Why）：确认项与命令投递都属于工作区实体状态，不涉及窗口或
    /// 其它领域；头部动作与键盘快捷键拿到实体句柄后直接调用，不再经外壳转手。
    pub(crate) fn run_compose(
        &mut self,
        name: String,
        action: ComposeAction,
        cx: &mut Context<Self>,
    ) {
        if action.is_dangerous() {
            self.state.modals.confirm = Some(ContainerConfirm::Compose(name));
            cx.notify();
            return;
        }
        let Some(project) = self
            .state
            .compose
            .iter()
            .find(|item| item.name == name)
            .cloned()
        else {
            return;
        };
        self.start_compose_project(project, action);
        cx.notify();
    }

    /// 投递编排命令并登记在途请求；调用方负责触发重绘。
    fn start_compose_project(&mut self, project: ComposeProject, action: ComposeAction) {
        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.notice =
            Some(rust_i18n::t!("container.compose.running", action = action.label()).to_string());
        self.send_tracked(
            request_id,
            ContainerCommand::RunCompose {
                tab_id: self.tab_id,
                request_id,
                project: Box::new(project),
                action,
            },
        );
    }

    /// 根据自定义或预设模板内容新建 Compose 项目并在详情中打开。
    pub(crate) fn create_local_compose_with_content(
        &mut self,
        content: String,
        cx: &mut Context<Self>,
    ) {
        let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
        let path = paths::compose_dir().join(format!("compose-{stamp}.yaml"));
        cx.spawn(async move |this, cx| {
            let create_message = rust_i18n::t!(
                "container.compose.create_failed",
                path = format!("{path:?}")
            )
            .to_string();
            let outcome = spawn_blocking_local(move || -> Result<PathBuf, String> {
                std::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("."))).map_err(
                    |error| {
                        rust_i18n::t!(
                            "container.compose.file_io_failed",
                            message = create_message.as_str(),
                            error = error
                        )
                        .to_string()
                    },
                )?;
                std::fs::write(&path, content).map_err(|error| {
                    rust_i18n::t!(
                        "container.compose.file_io_failed",
                        message = create_message.as_str(),
                        error = error
                    )
                    .to_string()
                })?;
                Ok(path)
            })
            .await;
            let _ = this.update(cx, |ws, cx| match outcome {
                Some(Ok(path)) => ws.open_local_compose(path, cx),
                Some(Err(error)) => {
                    ws.state.notice = Some(error);
                    cx.notify();
                }
                None => {
                    ws.state.notice =
                        Some(rust_i18n::t!("container.compose.file_task_failed").to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 导入外部 YAML 到托管目录，再启动为独立项目。
    pub(crate) fn import_local_compose(&mut self, cx: &mut Context<Self>) {
        let Some(source) = rfd::FileDialog::new()
            .add_filter("Compose YAML", &["yaml", "yml"])
            .pick_file()
        else {
            return;
        };
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .map(paths::sanitize_audit_label)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "compose.yaml".to_string());
        let directory = paths::compose_dir();
        let destination = directory.join(name);
        cx.spawn(async move |this, cx| {
            let import_message = rust_i18n::t!(
                "container.compose.import_failed",
                path = format!("{source:?}")
            )
            .to_string();
            let outcome = spawn_blocking_local(move || -> Result<PathBuf, String> {
                std::fs::create_dir_all(&directory).map_err(|error| {
                    rust_i18n::t!(
                        "container.compose.file_io_failed",
                        message = import_message.as_str(),
                        error = error
                    )
                    .to_string()
                })?;
                let destination = unique_compose_destination(&destination);
                std::fs::copy(&source, &destination).map_err(|error| {
                    rust_i18n::t!(
                        "container.compose.file_io_failed",
                        message = import_message.as_str(),
                        error = error
                    )
                    .to_string()
                })?;
                Ok(destination)
            })
            .await;
            let _ = this.update(cx, |ws, cx| match outcome {
                Some(Ok(path)) => ws.open_local_compose(path, cx),
                Some(Err(error)) => {
                    ws.state.notice = Some(error);
                    cx.notify();
                }
                None => {
                    ws.state.notice =
                        Some(rust_i18n::t!("container.compose.file_task_failed").to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_local_compose(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(name) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(paths::sanitize_audit_label)
        else {
            self.state.notice = Some(rust_i18n::t!("container.compose.name_invalid").to_string());
            cx.notify();
            return;
        };
        let project = ComposeProject {
            name: name.clone(),
            running: 0,
            total: 0,
            working_dir: path
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned()),
            config_files: vec![path.to_string_lossy().into_owned()],
        };
        if !self.state.compose.iter().any(|item| item.name == name) {
            self.state.compose.push(project);
        }
        self.state.section = ContainerSection::Compose;
        self.state.selected = Some(name.clone());
        self.state.detail_tab = ContainerDetailTab::Yaml;
        self.state.compose_yaml_editor = None;
        self.ensure_compose_yaml_loaded(cx);
        self.state.notice =
            Some(rust_i18n::t!("container.compose.created_hint", name = name).to_string());
        cx.notify();
    }

    /// 在内置编辑器中打开指定项目的 YAML，并自动定位到详情面板的 YAML 源码标签页。
    pub(crate) fn edit_compose_yaml(&mut self, name: &str, cx: &mut Context<Self>) {
        let project = self
            .state
            .compose
            .iter()
            .find(|item| item.name == name)
            .cloned();
        let Some(path) = project.as_ref().and_then(compose_project_yaml) else {
            self.state.notice = Some(rust_i18n::t!("container.compose.yaml_missing").to_string());
            cx.notify();
            return;
        };
        if !compose_yaml_is_allowed(&path, project.as_ref()) {
            self.state.notice =
                Some(rust_i18n::t!("container.compose.yaml_outside_root").to_string());
            cx.notify();
            return;
        }

        self.state.section = ContainerSection::Compose;
        self.state.selected = Some(name.to_string());
        self.state.detail_tab = ContainerDetailTab::Yaml;
        if let Some((current_name, _, _, _)) = &self.state.compose_yaml_editor
            && current_name != name
        {
            self.state.compose_yaml_editor = None;
        }
        self.ensure_compose_yaml_loaded(cx);
        cx.notify();
    }
}
