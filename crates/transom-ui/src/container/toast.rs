//! 请求动作的可读描述：结果事件用它拼右上角提示。
//!
//! 设计原理（Why）：控制平面的结果事件只带 `request_id` 与错误，而提示要说清
//! 「启动 web-1 成功」用户才知道刚点的那一下发生了什么。因此描述在**发送时**
//! 由命令本身推导（见 `AppShell::send_tracked`），而不是让每个调用点各自拼一句
//! ——那样漏一处就没有提示，文案也会四处漂移。
//!
//! 代价与局限（Trade-offs）：只读请求（列表、inspect、日志、统计）返回 `None`，
//! 成功时不弹提示，否则每次采样或列表刷新都会糊一条 toast；因此描述表只覆盖会改变
//! 引擎状态的动作。

use transom_core::{ContainerCommand, ContainerLifecycleAction};

/// 该命令成功后要提示的动作描述；只读命令返回 `None`。
pub(crate) fn command_toast_label(command: &ContainerCommand) -> Option<String> {
    use ContainerCommand as C;
    match command {
        C::Lifecycle {
            container, action, ..
        } => Some(verb_object(lifecycle_verb_key(action), container)),
        C::PullImage { reference, .. } => Some(verb_object("container.pull", reference)),
        C::RemoveImage { image, .. } => Some(verb_object("common.delete", image)),
        C::TagImage { image, repo, tag, .. } => {
            let target = match tag {
                Some(t) if !t.is_empty() => format!("{repo}:{t}"),
                _ => repo.clone(),
            };
            Some(verb_object("container.action.tag", &format!("{image} -> {target}")))
        }
        C::PushImage { image, tag, .. } => {
            let target = match tag {
                Some(t) if !t.is_empty() => format!("{image}:{t}"),
                _ => image.clone(),
            };
            Some(verb_object("container.action.push", &target))
        }
        C::BuildImage { spec, .. } => {
            let target = spec.tags.first().cloned().unwrap_or_else(|| spec.context_path.clone());
            Some(verb_object("container.action.build", &target))
        }
        C::SaveImage { image, .. } => {
            Some(verb_object("container.action.save_image", image))
        }
        C::LoadImage { local_path, .. } => {
            let name = local_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| local_path.display().to_string());
            Some(verb_object("container.action.load_image", &name))
        }
        C::CreateVolume { name, .. } => Some(verb_object("container.create", name)),
        C::CreateNetwork { spec, .. } => Some(verb_object("container.create", &spec.name)),
        C::RemoveVolume { name, .. } | C::RemoveNetwork { network: name, .. } => {
            Some(verb_object("common.delete", name))
        }
        C::ConnectNetwork { network, container, .. } => {
            Some(verb_object("container.action.connect_network", &format!("{container} -> {network}")))
        }
        C::DisconnectNetwork { network, container, .. } => {
            Some(verb_object("container.action.disconnect_network", &format!("{container} -/-> {network}")))
        }
        C::Rename { from, .. } => {
            Some(verb_object("container.action.rename", &path_basename(from)))
        }
        C::SaveFile { path, .. } | C::SaveVolumeFile { path, .. } => {
            Some(verb_object("common.save", &path_basename(path)))
        }
        C::RenameContainer { name, .. } => Some(verb_object("container.action.rename", name)),
        C::UpdateContainer { container, .. } => {
            Some(verb_object("container.action.update", container))
        }
        C::CommitContainer { container, repo, .. } => {
            Some(verb_object("container.action.commit", &format!("{container} -> {repo}")))
        }
        C::CreateContainer { spec, .. } => Some(verb_object(
            "container.run.title",
            if spec.name.is_empty() {
                spec.image.as_str()
            } else {
                spec.name.as_str()
            },
        )),
        C::CreateDir { path, .. } => {
            Some(verb_object("container.files.new_dir", &path_basename(path)))
        }
        C::Download { path, .. } => Some(verb_object(
            "container.files.download",
            &path_basename(path),
        )),
        C::Upload { path, .. } => Some(verb_object("container.files.upload", &path_basename(path))),
        C::RemovePath { path, .. } => Some(verb_object("common.delete", path)),
        C::RunCompose {
            project, action, ..
        } => {
            let key = match action {
                transom_core::ComposeAction::Up => "container.action.compose_up",
                transom_core::ComposeAction::Down => "container.action.compose_down",
                transom_core::ComposeAction::Restart => "container.action.compose_restart",
                transom_core::ComposeAction::Build => "container.action.compose_build",
                transom_core::ComposeAction::Pull => "container.action.compose_pull",
            };
            Some(verb_object(key, &project.name))
        }
        // 连接、列表、inspect、日志、统计、终端、取消：成功时不打扰用户。
        _ => None,
    }
}

/// 成功提示文案：例如「启动 web-1 成功」。
pub(crate) fn toast_succeeded(label: &str) -> String {
    rust_i18n::t!("container.toast.succeeded", action = label).to_string()
}

/// 失败提示文案：带上是哪件事失败；没有动作描述（只读请求）时只报错误本身。
pub(crate) fn toast_failed(label: Option<&str>, error: &str) -> String {
    match label {
        Some(label) => {
            rust_i18n::t!("container.toast.failed", action = label, error = error).to_string()
        }
        None => error.to_string(),
    }
}

/// 「动词 + 对象」的中文短语，例如 `启动 web-1`。
fn verb_object(verb_key: &str, object: &str) -> String {
    format!("{} {}", rust_i18n::t!(verb_key), object)
}

fn lifecycle_verb_key(action: &ContainerLifecycleAction) -> &'static str {
    match action {
        ContainerLifecycleAction::Start => "container.action.start",
        ContainerLifecycleAction::Stop => "container.action.stop",
        ContainerLifecycleAction::Restart => "container.action.restart",
        ContainerLifecycleAction::Pause => "container.action.pause",
        ContainerLifecycleAction::Unpause => "container.action.unpause",
        ContainerLifecycleAction::Kill { .. } => "container.action.kill",
        ContainerLifecycleAction::Remove { .. } => "common.delete",
    }
}

/// 取路径最后一段做提示对象：容器内路径是 POSIX 风格，不能交给宿主机的 `Path`。
fn path_basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{command_toast_label, path_basename};
    use transom_core::{
        ComposeAction, ComposeProject, ContainerCommand, ContainerLifecycleAction,
        ContainerRequestId,
    };

    fn command() -> ContainerCommand {
        ContainerCommand::Lifecycle {
            tab_id: transom_core::TabId::new(),
            request_id: ContainerRequestId::new(),
            container: "web-1".to_string(),
            action: ContainerLifecycleAction::Stop,
        }
    }

    #[test]
    fn mutating_commands_carry_a_description() {
        let label = command_toast_label(&command()).unwrap_or_default();
        assert!(label.contains("web-1"), "描述要带上对象，实际: {label}");
        assert!(
            label.starts_with(rust_i18n::t!("container.action.stop").as_ref()),
            "动词取自动作本身，实际: {label}"
        );

        let pull = ContainerCommand::PullImage {
            tab_id: transom_core::TabId::new(),
            request_id: ContainerRequestId::new(),
            reference: "busybox:1.36".to_string(),
        };
        assert!(
            command_toast_label(&pull)
                .unwrap_or_default()
                .contains("busybox:1.36")
        );
    }

    #[test]
    fn read_only_commands_stay_silent() {
        let list = ContainerCommand::ListContainers {
            tab_id: transom_core::TabId::new(),
            request_id: ContainerRequestId::new(),
        };
        assert!(command_toast_label(&list).is_none(), "刷新不该弹提示");
        let inspect = ContainerCommand::InspectContainer {
            tab_id: transom_core::TabId::new(),
            request_id: ContainerRequestId::new(),
            container: "web-1".to_string(),
        };
        assert!(command_toast_label(&inspect).is_none());
    }

    #[test]
    fn compose_description_follows_direction() {
        let project = ComposeProject {
            name: "demo".to_string(),
            running: 1,
            total: 2,
            working_dir: Some("/tmp/demo".to_string()),
            config_files: Vec::new(),
        };
        let down = ContainerCommand::RunCompose {
            tab_id: transom_core::TabId::new(),
            request_id: ContainerRequestId::new(),
            project: Box::new(project),
            action: ComposeAction::Down,
        };
        let label = command_toast_label(&down).unwrap_or_default();
        assert!(label.contains("demo"), "实际: {label}");
        assert!(
            label.starts_with(rust_i18n::t!("container.action.compose_down").as_ref()),
            "实际: {label}"
        );
    }

    #[test]
    fn basename_ignores_trailing_slash_and_keeps_unknown_paths() {
        assert_eq!(path_basename("/var/lib/data"), "data");
        assert_eq!(path_basename("/var/lib/data/"), "data");
        assert_eq!(path_basename("/"), "/");
        assert_eq!(path_basename("no-slash"), "no-slash");
    }
}
