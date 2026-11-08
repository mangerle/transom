//! 构建弹窗的打开、上下文选择与提交动作。

use super::super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{ContainerBuildSpec, ContainerCommand, ContainerRequestId, TabId};

impl AppShell {
    /// 打开从 Dockerfile 构建镜像弹窗。
    pub(crate) fn open_container_build_dialog(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.build.context_placeholder").to_string())
        });

        let dockerfile = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.build.dockerfile_placeholder").to_string())
                .default_value("Dockerfile".to_string())
        });

        let tag = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.build.tag_placeholder").to_string())
        });
        let target = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.build.target_placeholder").to_string())
        });
        let platform = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.build.platform_placeholder").to_string())
        });
        let build_args = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.build.build_args_placeholder").to_string())
        });

        let sub1 = cx.subscribe(&context_path, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_build_dialog(tab_id, cx);
            }
        });

        let sub2 = cx.subscribe(&dockerfile, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_build_dialog(tab_id, cx);
            }
        });

        let sub3 = cx.subscribe(&tag, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_build_dialog(tab_id, cx);
            }
        });
        let sub4 = cx.subscribe(&target, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_build_dialog(tab_id, cx);
            }
        });
        let sub5 = cx.subscribe(&platform, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_build_dialog(tab_id, cx);
            }
        });
        let sub6 = cx.subscribe(&build_args, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_build_dialog(tab_id, cx);
            }
        });

        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.build_dialog = Some(ContainerBuildDialog {
                context_path,
                dockerfile,
                tag,
                target,
                platform,
                build_args,
                no_cache: false,
                building: false,
                request_id: None,
                logs: Vec::new(),
                error: None,
                success: false,
                _subscriptions: vec![sub1, sub2, sub3, sub4, sub5, sub6],
            });
        });
        cx.notify();
    }

    /// 浏览并选取本地构建上下文目录。
    pub(crate) fn browse_container_build_context_path(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let path_str = folder.display().to_string();
        let input = self
            .with_container(tab_id, cx, |workspace| {
                workspace
                    .build_dialog
                    .as_ref()
                    .map(|build| build.context_path.clone())
            })
            .flatten();
        if let Some(input) = input {
            input.update(cx, |input, cx| {
                input.set_value(path_str, window, cx);
            });
        }
        cx.notify();
    }

    /// 提交镜像构建任务。
    pub(crate) fn submit_container_build_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let (context_path_val, dockerfile_val, tag_val, target_val, platform_val, build_args_val) = {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                return;
            };
            let workspace = &workspace.read(cx).state;
            let Some(build) = workspace.build_dialog.as_ref() else {
                return;
            };
            if build.building {
                return;
            }
            (
                build.context_path.read(cx).value().trim().to_string(),
                build.dockerfile.read(cx).value().trim().to_string(),
                build.tag.read(cx).value().trim().to_string(),
                build.target.read(cx).value().trim().to_string(),
                build.platform.read(cx).value().trim().to_string(),
                build.build_args.read(cx).value().trim().to_string(),
            )
        };

        if context_path_val.is_empty() {
            self.with_container_mut(tab_id, cx, |workspace| {
                if let Some(build) = workspace.build_dialog.as_mut() {
                    build.error = Some(rust_i18n::t!("container.build.empty_context").to_string());
                }
            });
            cx.notify();
            return;
        }

        if tag_val.is_empty() {
            self.with_container_mut(tab_id, cx, |workspace| {
                if let Some(build) = workspace.build_dialog.as_mut() {
                    build.error = Some(rust_i18n::t!("container.build.empty_tag").to_string());
                }
            });
            cx.notify();
            return;
        }

        let dockerfile_opt = if dockerfile_val.is_empty() {
            Some("Dockerfile".to_string())
        } else {
            Some(dockerfile_val)
        };
        let build_args = match parse_build_args(&build_args_val) {
            Ok(args) => args,
            Err(error) => {
                self.with_container_mut(tab_id, cx, |workspace| {
                    if let Some(build) = workspace.build_dialog.as_mut() {
                        build.error = Some(error);
                    }
                });
                cx.notify();
                return;
            }
        };

        let request_id = ContainerRequestId::new();
        let Some(no_cache) = self
            .with_container_mut(tab_id, cx, |workspace| {
                let build = workspace.build_dialog.as_mut()?;
                build.building = true;
                build.request_id = Some(request_id);
                build.error = None;
                build.logs.clear();
                build.success = false;
                Some(build.no_cache)
            })
            .flatten()
        else {
            return;
        };

        let spec = ContainerBuildSpec {
            context_path: context_path_val,
            dockerfile: dockerfile_opt,
            tags: vec![tag_val],
            no_cache,
            build_args,
            target: (!target_val.is_empty()).then_some(target_val),
            platform: (!platform_val.is_empty()).then_some(platform_val),
        };

        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::BuildImage {
                        tab_id,
                        request_id,
                        spec,
                    },
                );
            });
        }
        cx.notify();
    }
}

/// 解析 `KEY=VALUE,KEY2=VALUE2` 形式的构建参数；也接受换行分隔。
pub(super) fn parse_build_args(raw: &str) -> Result<HashMap<String, String>, String> {
    let mut args = HashMap::new();
    for entry in raw.split([',', '\n']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((key, value)) = entry.split_once('=') else {
            return Err(
                rust_i18n::t!("container.build.build_args_invalid", entry = entry).to_string(),
            );
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(
                rust_i18n::t!("container.build.build_args_empty_key", entry = entry).to_string(),
            );
        }
        args.insert(key.to_string(), value.trim().to_string());
    }
    Ok(args)
}
