//! Compose YAML 的定位、加载与保存。

use super::super::*;
use crate::app::spawn_blocking_local;
use chrono::Local;
use gpui_kit::component::input::TextareaState;
use std::path::{Path, PathBuf};
use transom_core::ComposeProject;
use transom_platform::paths;

impl AppShell {
    /// 确保当前 Compose 项目的 YAML 内容已载入编辑器。
    pub(crate) fn ensure_compose_yaml_loaded(
        &mut self,
        tab_id: TabId,
        project_name: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .with_container(tab_id, cx, |workspace| {
                if let Some((current_name, _, _, _)) = &workspace.compose_yaml_editor
                    && current_name == project_name
                {
                    return None;
                }
                workspace
                    .compose
                    .iter()
                    .find(|p| p.name == project_name)
                    .cloned()
            })
            .flatten()
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
            let _ = this.update_in(cx, |this, window, cx| match content {
                Some(Ok(text)) => {
                    let parsed =
                        crate::container::detail::compose_graph::parse_compose_dependencies(&text);
                    let editor = cx.new(|cx| TextareaState::new(window, cx).default_value(text));
                    this.with_container_mut(tab_id, cx, |workspace| {
                        workspace.compose_yaml_editor =
                            Some((target_project, target_path, editor, false));
                        workspace.compose_dependencies = parsed;
                    });
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
            });
        })
        .detach();
    }

    /// 保存当前在编辑器中修改的 Compose YAML 内容。
    pub(crate) fn save_compose_yaml(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((path, editor, saving)) = self
            .with_container(tab_id, cx, |workspace| {
                let (_, path, editor, saving) = workspace.compose_yaml_editor.as_ref()?;
                Some((path.clone(), editor.clone(), *saving))
            })
            .flatten()
        else {
            return;
        };
        if saving {
            return;
        }
        self.with_container_mut(tab_id, cx, |workspace| {
            if let Some((_, _, _, saving)) = &mut workspace.compose_yaml_editor {
                *saving = true;
            }
        });
        let text = editor.read(cx).value().to_string();
        let write_text = text.clone();
        let write_path = path.clone();

        cx.spawn(async move |this, cx| {
            let result =
                spawn_blocking_local(move || std::fs::write(&write_path, write_text)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.with_container_mut(tab_id, cx, |workspace| {
                    if let Some((_, _, _, saving)) = &mut workspace.compose_yaml_editor {
                        *saving = false;
                    }
                });
                match result {
                    Some(Ok(())) => {
                        let parsed =
                            crate::container::detail::compose_graph::parse_compose_dependencies(
                                &text,
                            );
                        this.with_container_mut(tab_id, cx, |workspace| {
                            workspace.compose_dependencies = parsed;
                        });
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

    /// 确保当前 Compose 项目的微服务依赖图已解析载入。
    pub(crate) fn ensure_compose_dependencies_loaded(
        &mut self,
        tab_id: TabId,
        project_name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((dependencies_empty, cached_editor, project)) =
            self.with_container(tab_id, cx, |workspace| {
                let cached_editor = workspace
                    .compose_yaml_editor
                    .as_ref()
                    .filter(|(current_name, ..)| current_name == project_name)
                    .map(|(_, _, editor, _)| editor.clone());
                let project = workspace
                    .compose
                    .iter()
                    .find(|p| p.name == project_name)
                    .cloned();
                (
                    workspace.compose_dependencies.is_empty(),
                    cached_editor,
                    project,
                )
            })
        else {
            return;
        };
        if !dependencies_empty {
            return;
        }
        if let Some(editor) = cached_editor {
            let text = editor.read(cx).value();
            let parsed = crate::container::detail::compose_graph::parse_compose_dependencies(&text);
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.compose_dependencies = parsed;
            });
            return;
        }
        let Some(project) = project else {
            return;
        };
        let Some(yaml_path) = compose_project_yaml(&project) else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let content = spawn_blocking_local(move || std::fs::read_to_string(&yaml_path)).await;
            let _ = this.update_in(cx, |this, _, cx| {
                if let Some(Ok(text)) = content {
                    let parsed =
                        crate::container::detail::compose_graph::parse_compose_dependencies(&text);
                    this.with_container_mut(tab_id, cx, |workspace| {
                        workspace.compose_dependencies = parsed;
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

pub(super) fn unique_compose_destination(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("compose");
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("yaml");
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    path.with_file_name(format!("{stem}-{stamp}.{extension}"))
}

pub(super) fn compose_project_yaml(project: &ComposeProject) -> Option<PathBuf> {
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

pub(super) fn compose_yaml_is_allowed(path: &Path, project: Option<&ComposeProject>) -> bool {
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

pub(super) fn is_yaml_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "yaml" | "yml"))
}
