//! 从 Dockerfile 构建镜像（Build）向导弹窗。
//!
//! 设计原理（Why）：
//! - 将镜像构建的核心配置（构建上下文目录、Dockerfile 路径、镜像标签与免缓存开关）收敛为结构化向导；
//! - 集成本地原生目录选择器，免去手动输入复杂绝对路径的繁琐操作；
//! - 内嵌等宽日志预览流，实时消费来自后端的 `ContainerAppEvent::BuildProgress`，直观呈现分层构建全过程。

use super::*;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
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

        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.build_dialog = Some(ContainerBuildDialog {
                context_path,
                dockerfile,
                tag,
                no_cache: false,
                building: false,
                request_id: None,
                logs: Vec::new(),
                error: None,
                success: false,
                _subscriptions: vec![sub1, sub2, sub3],
            });
        }
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if let Some(build) = workspace.build_dialog.as_mut() {
                build.context_path.update(cx, |input, cx| {
                    input.set_value(path_str, window, cx);
                });
            }
        }
        cx.notify();
    }

    /// 提交镜像构建任务。
    pub(crate) fn submit_container_build_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let (context_path_val, dockerfile_val, tag_val) = {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                return;
            };
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
            )
        };

        if context_path_val.is_empty() {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                if let Some(build) = workspace.build_dialog.as_mut() {
                    build.error = Some(rust_i18n::t!("container.build.empty_context").to_string());
                }
            }
            cx.notify();
            return;
        }

        if tag_val.is_empty() {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                if let Some(build) = workspace.build_dialog.as_mut() {
                    build.error = Some(rust_i18n::t!("container.build.empty_tag").to_string());
                }
            }
            cx.notify();
            return;
        }

        let dockerfile_opt = if dockerfile_val.is_empty() {
            Some("Dockerfile".to_string())
        } else {
            Some(dockerfile_val)
        };

        let request_id = ContainerRequestId::new();
        let no_cache = {
            let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
                return;
            };
            let Some(build) = workspace.build_dialog.as_mut() else {
                return;
            };
            build.building = true;
            build.request_id = Some(request_id);
            build.error = None;
            build.logs.clear();
            build.success = false;
            build.no_cache
        };

        let spec = ContainerBuildSpec {
            context_path: context_path_val,
            dockerfile: dockerfile_opt,
            tags: vec![tag_val],
            no_cache,
            ..Default::default()
        };

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::BuildImage {
                tab_id,
                request_id,
                spec,
            },
        );
        cx.notify();
    }

    /// 渲染镜像构建弹窗主体。
    pub(crate) fn render_container_build_dialog_body(
        &self,
        tab_id: TabId,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let Some(build) = workspace.build_dialog.as_ref() else {
            return div().into_any_element();
        };

        let context_path = build.context_path.clone();
        let dockerfile = build.dockerfile.clone();
        let tag = build.tag.clone();
        let no_cache = build.no_cache;
        let building = build.building;
        let logs = build.logs.clone();
        let error = build.error.clone();

        let success = build.success;
        let tag_display = build.tag.read(cx).value().to_string();

        let app_browse = app.clone();
        let app_cancel = app.clone();
        let app_submit = app.clone();
        let app_toggle_cache = app.clone();

        v_flex()
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.build.context_path").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&context_path).w_full())
                            .child(
                                Button::new("container-build-browse")
                                    .secondary()
                                    .small()
                                    .label(rust_i18n::t!("container.build.browse").to_string())
                                    .on_click(move |_, window, cx| {
                                        app_browse.update(cx, |this, cx| {
                                            this.browse_container_build_context_path(
                                                tab_id, window, cx,
                                            );
                                        });
                                    }),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("container.build.dockerfile").to_string()),
                            )
                            .child(Input::new(&dockerfile).w_full()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("container.build.tag").to_string()),
                            )
                            .child(Input::new(&tag).w_full()),
                    ),
            )
            .child(
                h_flex()
                    .id("container-build-no-cache")
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        app_toggle_cache.update(cx, |this, cx| {
                            if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                if let Some(b) = ws.build_dialog.as_mut() {
                                    b.no_cache = !b.no_cache;
                                }
                            }
                            cx.notify();
                        });
                    })
                    .child(
                        Icon::new(if no_cache {
                            IconName::Check
                        } else {
                            IconName::Square
                        })
                        .xsmall()
                        .text_color(if no_cache {
                            p.primary
                        } else {
                            p.muted_foreground
                        }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.build.no_cache").to_string()),
                    ),
            )
            .when(building, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Loader).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.primary)
                                .child(rust_i18n::t!("container.build.building").to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(crate::widgets::progress_bar(1.0, p.primary, p)),
                        ),
                )
            })
            .when(!logs.is_empty() || building, |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.build.logs_title").to_string()),
                        )
                        .child(
                            div()
                                .id("container-build-logs-scroll")
                                .p_2()
                                .rounded(p.radius.lg)
                                .bg(p.muted.opacity(0.04))
                                .border_1()
                                .border_color(p.border.opacity(0.18))
                                .text_color(p.foreground)
                                .font_family(p.mono_font_family.clone())
                                .h(px(140.0))
                                .overflow_y_scroll()
                                .children(if logs.is_empty() {
                                    vec![
                                        div()
                                            .text_xs()
                                            .text_color(p.muted_foreground)
                                            .child(
                                                rust_i18n::t!("container.build.waiting_logs")
                                                    .to_string(),
                                            )
                                            .into_any_element(),
                                    ]
                                } else {
                                    logs.into_iter()
                                        .map(|line| div().text_xs().child(line).into_any_element())
                                        .collect()
                                }),
                        ),
                )
            })
            .when(success, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .items_center()
                        .rounded(p.radius.lg)
                        .bg(p.success.opacity(0.12))
                        .text_color(p.success)
                        .child(Icon::new(IconName::Check).small())
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(rust_i18n::t!("container.build.success", tag = tag_display)),
                        ),
                )
            })
            .when_some(error, |this, err| {
                this.child(
                    h_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .items_center()
                        .rounded(p.radius.lg)
                        .bg(p.danger.opacity(0.12))
                        .text_color(p.danger)
                        .child(Icon::new(IconName::TriangleAlert).small())
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(rust_i18n::t!("container.build.failed", error = err)),
                        ),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("container-build-cancel")
                            .ghost()
                            .small()
                            .label(rust_i18n::t!("common.cancel").to_string())
                            .on_click(move |_, _, cx| {
                                app_cancel.update(cx, |this, cx| {
                                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                        ws.build_dialog = None;
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("container-build-submit")
                            .primary()
                            .small()
                            .disabled(building)
                            .label(if building {
                                rust_i18n::t!("container.build.building").to_string()
                            } else {
                                rust_i18n::t!("container.build.start_build").to_string()
                            })
                            .on_click(move |_, _, cx| {
                                app_submit.update(cx, |this, cx| {
                                    this.submit_container_build_dialog(tab_id, cx);
                                });
                            }),
                    ),
            )
            .into_any_element()
    }
}
