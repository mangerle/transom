//! Compose YAML 的定位、加载与保存。

use super::super::*;
use crate::app::spawn_blocking_local;
use chrono::Local;
use std::path::{Path, PathBuf};
use transom_core::ComposeProject;
use transom_platform::paths;

impl ContainerWorkspace {
    /// 保存当前在编辑器中修改的 Compose YAML 内容。
    ///
    /// 设计原理（Why）：编辑器实体与配置文件路径都在工作区状态里，保存不需要
    /// 窗口；完成提示写入实体 `notice`，由外壳渲染帧的 `flush_container_notice`
    /// 统一转成通知，避免为一条回执再把动作转回外壳。
    pub(crate) fn save_compose_yaml(&mut self, cx: &mut Context<Self>) {
        let Some((_, path, editor, saving)) = self.state.compose_yaml_editor.as_ref() else {
            return;
        };
        if *saving {
            return;
        }
        // 先取出所需的拥有型数据，避免后续异步闭包与状态借用冲突。
        let path = path.clone();
        let editor = editor.clone();
        if let Some((_, _, _, saving)) = &mut self.state.compose_yaml_editor {
            *saving = true;
        }
        let text = editor.read(cx).value().to_string();
        let write_text = text.clone();
        let write_path = path.clone();

        cx.spawn(async move |this, cx| {
            let result =
                spawn_blocking_local(move || std::fs::write(&write_path, write_text)).await;
            let _ = this.update(cx, |workspace, cx| {
                if let Some((_, _, _, saving)) = &mut workspace.state.compose_yaml_editor {
                    *saving = false;
                }
                match result {
                    Some(Ok(())) => {
                        workspace.state.compose_dependencies =
                            crate::container::detail::compose_graph::parse_compose_dependencies(
                                &text,
                            );
                        workspace.state.notice =
                            Some(rust_i18n::t!("container.compose.yaml_saved").to_string());
                    }
                    Some(Err(error)) => {
                        workspace.state.notice = Some(
                            rust_i18n::t!("container.compose.save_failed", error = error)
                                .to_string(),
                        );
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl ContainerWorkspace {
    /// 确保当前选中的 Compose 项目 YAML 内容已载入编辑器。
    ///
    /// 设计原理（Why）：读盘与解析都属于工作区实体，异步任务完成后把文本通过
    /// 事件带回外壳；`TextareaState` 的创建需要 `Window`，由外壳在渲染期完成，
    /// 实体不在派发阶段同步开窗，也不把 `Window` 传进纯渲染链路。
    ///
    /// 代价与局限：编辑器就绪晚于文件读取一帧；`compose_yaml_loading` 只是同一
    /// 项目的在途闸门，不缓存读取失败结果，失败后再次进入页签会重试。
    pub(crate) fn ensure_compose_yaml_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.state.selected_compose().cloned() else {
            return;
        };
        if self
            .state
            .compose_yaml_editor
            .as_ref()
            .is_some_and(|(name, ..)| name == &project.name)
        {
            return;
        }
        if self.state.compose_yaml_loading.as_deref() == Some(project.name.as_str()) {
            return;
        }
        let Some(yaml_path) = compose_project_yaml(&project) else {
            return;
        };

        self.state.compose_yaml_loading = Some(project.name.clone());
        let project_name = project.name.clone();
        let target_path = yaml_path.clone();
        cx.spawn(async move |this, cx| {
            let read_path = target_path.clone();
            let content = spawn_blocking_local(move || std::fs::read_to_string(&read_path)).await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.state.compose_yaml_loading = None;
                match content {
                    Some(Ok(text)) => {
                        workspace.state.compose_dependencies =
                            crate::container::detail::compose_graph::parse_compose_dependencies(
                                &text,
                            );
                        cx.emit(ContainerWorkspaceEvent::ComposeYamlLoaded {
                            project: project_name,
                            path: target_path,
                            text,
                        });
                    }
                    Some(Err(error)) => {
                        cx.emit(ContainerWorkspaceEvent::ComposeYamlReadFailed(
                            rust_i18n::t!("container.compose.read_failed", error = error)
                                .to_string(),
                        ));
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 确保当前 Compose 项目的微服务依赖图已解析载入。
    ///
    /// 设计原理（Why）：已有 YAML 编辑器时直接读取编辑器文本解析，避免重复读盘；
    /// 没有编辑器时才异步读取配置文件。依赖图属于工作区状态，后续拓扑页直接消费。
    ///
    /// 代价与局限：解析在异步任务里完成，失败与空依赖图在界面上不可区分；这与
    /// 原有行为一致，依赖数据本身没有独立的错误出口。
    pub(crate) fn ensure_compose_dependencies_loaded(&mut self, cx: &mut Context<Self>) {
        if !self.state.compose_dependencies.is_empty() {
            return;
        }
        let Some(project_name) = self.state.selected_compose().map(|p| p.name.clone()) else {
            return;
        };
        if self.state.compose_dependencies_loading.as_deref() == Some(project_name.as_str()) {
            return;
        }
        let cached_editor = self
            .state
            .compose_yaml_editor
            .as_ref()
            .filter(|(name, ..)| name == &project_name)
            .map(|(_, _, editor, _)| editor.clone());
        if let Some(editor) = cached_editor {
            let text = editor.read(cx).value();
            self.state.compose_dependencies =
                crate::container::detail::compose_graph::parse_compose_dependencies(&text);
            return;
        }

        let Some(project) = self.state.selected_compose().cloned() else {
            return;
        };
        let Some(yaml_path) = compose_project_yaml(&project) else {
            return;
        };
        self.state.compose_dependencies_loading = Some(project_name);
        cx.spawn(async move |this, cx| {
            let content = spawn_blocking_local(move || std::fs::read_to_string(&yaml_path)).await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.state.compose_dependencies_loading = None;
                if let Some(Ok(text)) = content {
                    workspace.state.compose_dependencies =
                        crate::container::detail::compose_graph::parse_compose_dependencies(&text);
                }
                cx.notify();
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
