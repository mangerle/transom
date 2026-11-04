//! 编排项目的起停动作。

use super::*;
use crate::sftp::spawn_blocking_local;
use chrono::Local;
use gpui_kit::component::input::TextareaState;
use std::path::{Path, PathBuf};
use transom_core::ComposeProject;
use transom_platform::paths;

const DEFAULT_COMPOSE_YAML: &str = "services:\n  app:\n    image: alpine:3.20\n    command: [\"sh\", \"-c\", \"while true; do sleep 3600; done\"]\n";

impl AppShell {
    /// 编排项目命令：`up` 直接下发，`down` 会删容器与网络，先过确认条。
    pub(crate) fn run_compose(
        &mut self,
        tab_id: TabId,
        name: String,
        action: transom_core::ComposeAction,
        cx: &mut Context<Self>,
    ) {
        if action.is_dangerous() {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.confirm = Some(ContainerConfirm::Compose(name));
            }
            cx.notify();
            return;
        }
        let Some(project) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| {
                workspace
                    .compose
                    .iter()
                    .find(|item| item.name == name)
                    .cloned()
            })
        else {
            return;
        };
        self.start_compose_project(tab_id, project, action, cx);
    }

    /// 新建一个受应用托管的 Compose YAML，并立即以项目名启动。
    pub(crate) fn create_local_compose(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
        let path = paths::compose_dir().join(format!("compose-{stamp}.yaml"));
        let body = DEFAULT_COMPOSE_YAML.to_string();
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
                std::fs::write(&path, body).map_err(|error| {
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
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                Some(Ok(path)) => this.open_local_compose(tab_id, path, window, cx),
                Some(Err(error)) => this.show_toast(error, false, window, cx),
                None => this.show_toast(
                    rust_i18n::t!("container.compose.file_task_failed").to_string(),
                    false,
                    window,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// 导入外部 YAML 到托管目录，再启动为独立项目。
    pub(crate) fn import_local_compose(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
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
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                Some(Ok(path)) => this.open_local_compose(tab_id, path, window, cx),
                Some(Err(error)) => this.show_toast(error, false, window, cx),
                None => this.show_toast(
                    rust_i18n::t!("container.compose.file_task_failed").to_string(),
                    false,
                    window,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// 用系统默认编辑器打开当前项目的 YAML，路径必须位于受控目录或工作目录内。
    pub(crate) fn edit_compose_yaml(
        &mut self,
        tab_id: TabId,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.compose.iter().find(|item| item.name == name))
            .cloned();
        let Some(path) = project.as_ref().and_then(compose_project_yaml) else {
            self.show_toast(
                rust_i18n::t!("container.compose.yaml_missing").to_string(),
                false,
                window,
                cx,
            );
            return;
        };
        if !compose_yaml_is_allowed(&path, project.as_ref()) {
            self.show_toast(
                rust_i18n::t!("container.compose.yaml_outside_root").to_string(),
                false,
                window,
                cx,
            );
            return;
        }
        crate::audit::open_local_path(&path);
    }

    fn open_local_compose(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(paths::sanitize_audit_label)
        else {
            self.show_toast(
                rust_i18n::t!("container.compose.name_invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        };
        let project = ComposeProject {
            name,
            running: 0,
            total: 0,
            working_dir: path
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned()),
            config_files: vec![path.to_string_lossy().into_owned()],
        };
        self.start_compose_project(tab_id, project, transom_core::ComposeAction::Up, cx);
        crate::audit::open_local_path(&path);
    }

    fn start_compose_project(
        &mut self,
        tab_id: TabId,
        project: ComposeProject,
        action: transom_core::ComposeAction,
        cx: &mut Context<Self>,
    ) {
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.notice = Some(
                rust_i18n::t!("container.compose.running", action = action.label()).to_string(),
            );
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::RunCompose {
                tab_id,
                request_id,
                project: Box::new(project),
                action,
            },
        );
        cx.notify();
    }

    /// 确保当前 Compose 项目的 YAML 内容已载入编辑器。
    pub(crate) fn ensure_compose_yaml_loaded(
        &mut self,
        tab_id: TabId,
        project_name: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if let Some((current_name, _, _, _)) = &workspace.compose_yaml_editor {
            if current_name == project_name {
                return;
            }
        }
        let Some(project) = workspace
            .compose
            .iter()
            .find(|p| p.name == project_name)
            .cloned()
        else {
            return;
        };
        let Some(yaml_path) = compose_project_yaml(&project) else {
            return;
        };

        let target_project = project_name.to_string();
        let target_path = yaml_path.clone();
        cx.spawn(async move |this, cx| {
            let read_path = target_path.clone();
            let content = spawn_blocking_local(move || std::fs::read_to_string(&read_path)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(workspace) = this.state.container_workspace_mut(tab_id) else {
                    return;
                };
                match content {
                    Some(Ok(text)) => {
                        let editor =
                            cx.new(|cx| TextareaState::new(window, cx).default_value(text));
                        workspace.compose_yaml_editor =
                            Some((target_project, target_path, editor, false));
                        cx.notify();
                    }
                    Some(Err(err)) => {
                        this.show_toast(
                            rust_i18n::t!("container.compose.read_failed", error = err).to_string(),
                            false,
                            window,
                            cx,
                        );
                    }
                    None => {}
                }
            });
        })
        .detach();
    }

    /// 保存当前在编辑器中修改的 Compose YAML 内容。
    pub(crate) fn save_compose_yaml(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some((_, path, editor, saving)) = &mut workspace.compose_yaml_editor else {
            return;
        };
        if *saving {
            return;
        }
        *saving = true;
        let text = editor.read(cx).value().to_string();
        let write_path = path.clone();

        cx.spawn(async move |this, cx| {
            let result = spawn_blocking_local(move || std::fs::write(&write_path, text)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                    if let Some((_, _, _, saving)) = &mut workspace.compose_yaml_editor {
                        *saving = false;
                    }
                }
                match result {
                    Some(Ok(())) => {
                        this.show_toast(
                            rust_i18n::t!("container.compose.yaml_saved").to_string(),
                            true,
                            window,
                            cx,
                        );
                    }
                    Some(Err(err)) => {
                        this.show_toast(
                            rust_i18n::t!("container.compose.save_failed", error = err).to_string(),
                            false,
                            window,
                            cx,
                        );
                    }
                    None => {}
                }
            });
        })
        .detach();
        cx.notify();
    }
}

fn unique_compose_destination(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("compose");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("yaml");
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    path.with_file_name(format!("{stem}-{stamp}.{extension}"))
}

pub(crate) fn compose_project_yaml(project: &ComposeProject) -> Option<PathBuf> {
    for path in &project.config_files {
        let path = PathBuf::from(path);
        if is_yaml_file(&path) && path.is_file() {
            return Some(path);
        }
    }
    let directory = PathBuf::from(project.working_dir.as_deref()?);
    [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ]
    .into_iter()
    .map(|name| directory.join(name))
    .find(|path| path.is_file())
}

fn compose_yaml_is_allowed(path: &Path, project: Option<&ComposeProject>) -> bool {
    let Some(canonical) = path.canonicalize().ok() else {
        return false;
    };
    let managed = paths::compose_dir()
        .canonicalize()
        .ok()
        .is_some_and(|root| canonical.starts_with(root));
    if managed {
        return true;
    }
    project
        .and_then(|project| project.working_dir.as_deref())
        .and_then(|working_dir| PathBuf::from(working_dir).canonicalize().ok())
        .is_some_and(|root| canonical.starts_with(root))
}

fn is_yaml_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "yaml" | "yml"))
}
