//! 构建弹窗的打开、上下文选择与提交动作。
//!
//! 动作归属（Why）：构建参数、日志与在途请求都随标签页工作区存活，打开、
//! 选择上下文与提交只读写这些状态并投递命令，因此全部落在
//! [`ContainerWorkspace`] 上；外壳只把「打开构建弹窗」的窗口级意图转成实体更新。

use super::super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{ContainerBuildSpec, ContainerCommand, ContainerRequestId};

impl ContainerWorkspace {
    /// 打开从 Dockerfile 构建镜像弹窗。
    pub(crate) fn open_build_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

        let sub1 = cx.subscribe(&context_path, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_build_dialog(cx);
            }
        });
        let sub2 = cx.subscribe(&dockerfile, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_build_dialog(cx);
            }
        });
        let sub3 = cx.subscribe(&tag, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_build_dialog(cx);
            }
        });
        let sub4 = cx.subscribe(&target, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_build_dialog(cx);
            }
        });
        let sub5 = cx.subscribe(&platform, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_build_dialog(cx);
            }
        });
        let sub6 = cx.subscribe(&build_args, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_build_dialog(cx);
            }
        });

        self.state.build_dialog = Some(ContainerBuildDialog {
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
        cx.notify();
    }

    /// 浏览并选取本地构建上下文目录。
    ///
    /// 文件选择器是阻塞式系统调用，但只在用户点击时触发一次，与迁移前一致；
    /// 选中后直接写入上下文输入框。
    pub(crate) fn browse_build_context_path(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let path_str = folder.display().to_string();
        let Some(input) = self
            .state
            .build_dialog
            .as_ref()
            .map(|build| build.context_path.clone())
        else {
            return;
        };
        input.update(cx, |input, cx| {
            input.set_value(path_str, window, cx);
        });
        cx.notify();
    }

    /// 关闭构建弹窗。
    pub(crate) fn close_build_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.build_dialog = None;
        cx.notify();
    }

    /// 切换「不使用缓存」选项。
    pub(crate) fn toggle_build_no_cache(&mut self, cx: &mut Context<Self>) {
        if let Some(build) = self.state.build_dialog.as_mut() {
            build.no_cache = !build.no_cache;
        }
        cx.notify();
    }

    /// 提交镜像构建任务。
    ///
    /// 校验失败时就地写回错误并保持弹窗；成功后标记构建中、清空旧日志并
    /// 投递命令，流式日志与结果事件由外壳转发回实体。
    pub(crate) fn submit_build_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(build) = self.state.build_dialog.as_ref() else {
            return;
        };
        if build.building {
            return;
        }
        let context_path_val = build.context_path.read(cx).value().trim().to_string();
        let dockerfile_val = build.dockerfile.read(cx).value().trim().to_string();
        let tag_val = build.tag.read(cx).value().trim().to_string();
        let target_val = build.target.read(cx).value().trim().to_string();
        let platform_val = build.platform.read(cx).value().trim().to_string();
        let build_args_val = build.build_args.read(cx).value().trim().to_string();

        if context_path_val.is_empty() {
            self.set_build_error(
                rust_i18n::t!("container.build.empty_context").to_string(),
                cx,
            );
            return;
        }
        if tag_val.is_empty() {
            self.set_build_error(rust_i18n::t!("container.build.empty_tag").to_string(), cx);
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
                self.set_build_error(error, cx);
                return;
            }
        };

        let request_id = ContainerRequestId::new();
        let Some(no_cache) = self.state.build_dialog.as_mut().map(|build| {
            build.building = true;
            build.request_id = Some(request_id);
            build.error = None;
            build.logs.clear();
            build.success = false;
            build.no_cache
        }) else {
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

        self.send_tracked(
            request_id,
            ContainerCommand::BuildImage {
                tab_id: self.tab_id,
                request_id,
                spec,
            },
        );
        cx.notify();
    }

    /// 写入构建弹窗错误并触发重绘。
    fn set_build_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(build) = self.state.build_dialog.as_mut() {
            build.error = Some(error);
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
