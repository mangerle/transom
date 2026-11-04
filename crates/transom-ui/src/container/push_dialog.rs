//! 镜像推送到远端仓库（Docker Hub / Harbor）弹窗界面。
//!
//! 设计原理（Why）：
//! - 将推送镜像与已配置的 `ContainerRegistry` 凭证无缝联动，用户只需下拉选择目标源；
//! - 自动根据所选仓库域名与源镜像 Tag 智能补全目标 Tag 前缀与版本；
//! - 自动注入 Base64 AuthConfig 凭证头部，彻底解决私有仓库 401/403 鉴权失败问题。

use super::*;
use super::list::dropdown_trigger;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ContainerCommand, ContainerRegistry, ContainerRequestId, TabId};

impl AppShell {
    /// 打开推送到远端镜像仓库弹窗。
    pub(crate) fn open_container_push_dialog(
        &mut self,
        tab_id: TabId,
        image_id: String,
        source_tag: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let registries = app_settings(cx).container_registries.clone();
        let default_reg = registries
            .iter()
            .find(|r| r.is_default)
            .cloned()
            .unwrap_or_else(ContainerRegistry::docker_hub);

        // 拆解源镜像名称与 tag（例如 "nginx:alpine" -> ("nginx", "alpine")）
        let (repo_part, tag_part) = if let Some((repo, tag)) = source_tag.split_once(':') {
            (repo.to_string(), tag.to_string())
        } else {
            (source_tag.clone(), "latest".to_string())
        };

        let initial_repo = if default_reg.id == "docker-hub" {
            repo_part
        } else {
            format!("{}/{}", default_reg.domain(), repo_part)
        };

        let target_repo = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.push.target_repo_placeholder").to_string())
                .default_value(initial_repo)
        });

        let target_tag = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.push.target_tag_placeholder").to_string())
                .default_value(tag_part)
        });

        let sub1 = cx.subscribe(&target_repo, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_push_dialog(tab_id, cx);
            }
        });

        let sub2 = cx.subscribe(&target_tag, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_container_push_dialog(tab_id, cx);
            }
        });

        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.push_dialog = Some(ContainerPushDialog {
                image_id,
                source_tag,
                selected_registry_id: default_reg.id,
                target_repo,
                target_tag,
                error: None,
                _subscriptions: vec![sub1, sub2],
            });
        }
        cx.notify();
    }

    /// 提交推送到远端请求。
    pub(crate) fn submit_container_push_dialog(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let (image_id, repo, tag, registry) = {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                return;
            };
            let Some(push) = workspace.push_dialog.as_ref() else {
                return;
            };
            let repo = push.target_repo.read(cx).value().trim().to_string();
            let tag = push.target_tag.read(cx).value().trim().to_string();
            let reg = app_settings(cx)
                .container_registries
                .iter()
                .find(|r| r.id == push.selected_registry_id)
                .cloned()
                .unwrap_or_else(ContainerRegistry::docker_hub);
            (push.image_id.clone(), repo, tag, reg)
        };

        if repo.is_empty() || tag.is_empty() {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                if let Some(push) = workspace.push_dialog.as_mut() {
                    push.error = Some(
                        rust_i18n::t!("container.push.repo_and_tag_required").to_string(),
                    );
                }
            }
            cx.notify();
            return;
        }

        // 构造鉴权凭证
        let auth = if !registry.username.is_empty() {
            Some(transom_core::ContainerRegistryAuth {
                username: registry.username,
                password: registry.password,
                server_address: Some(registry.url),
                identity_token: None,
            })
        } else {
            None
        };

        let target_full_ref = format!("{repo}:{tag}");
        let request_id = ContainerRequestId::new();

        // 如果目标镜像名和源镜像名不同，先执行打标签
        let tag_request_id = ContainerRequestId::new();
        self.send_tracked(
            tab_id,
            tag_request_id,
            ContainerCommand::TagImage {
                tab_id,
                request_id: tag_request_id,
                image: image_id,
                repo: repo.clone(),
                tag: Some(tag.clone()),
            },
        );

        // 紧接着发起推送
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::PushImage {
                tab_id,
                request_id,
                image: repo,
                tag: Some(tag),
                auth,
            },
        );

        // 关闭推送弹窗并记录通知
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.push_dialog = None;
            workspace.notice = Some(
                rust_i18n::t!(
                    "container.push.pushing_notice",
                    reference = target_full_ref
                )
                .to_string(),
            );
        }
        cx.notify();
    }

    /// 渲染推送到远端弹窗主体。
    pub(crate) fn render_container_push_dialog_body(
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
        let Some(push) = workspace.push_dialog.as_ref() else {
            return div().into_any_element();
        };

        let source_tag = push.source_tag.clone();
        let selected_reg_id = push.selected_registry_id.clone();
        let registries = app_settings(cx).container_registries.clone();
        let target_repo = push.target_repo.clone();
        let target_tag = push.target_tag.clone();
        let error = push.error.clone();

        let current_registry = registries
            .iter()
            .find(|r| r.id == selected_reg_id)
            .cloned()
            .unwrap_or_else(ContainerRegistry::docker_hub);

        let app_menu = app.clone();
        let reg_menu = dropdown_trigger(
            "container-push-registry-select",
            current_registry.name.clone(),
            IconName::ChevronDown,
            None,
            move |menu, _, _| {
                let mut menu = menu;
                for reg in &registries {
                    let app_item = app_menu.clone();
                    let reg_item = reg.clone();
                    menu = menu.item(
                        PopupMenuItem::new(reg.name.clone())
                            .checked(reg.id == selected_reg_id)
                            .on_click(move |_, window, cx| {
                                app_item.update(cx, |this, cx| {
                                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                        if let Some(p) = ws.push_dialog.as_mut() {
                                            p.selected_registry_id = reg_item.id.clone();
                                            // 自动根据所选仓库域名补全前缀
                                            let current_val = p.target_repo.read(cx).value();
                                            let pure_name = current_val.as_ref().rsplit('/').next().unwrap_or(current_val.as_ref());
                                            let new_val = if reg_item.id == "docker-hub" {
                                                pure_name.to_string()
                                            } else {
                                                format!("{}/{}", reg_item.domain(), pure_name)
                                            };
                                            p.target_repo.update(cx, |input, cx| {
                                                input.set_value(new_val, window, cx);
                                            });
                                        }
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            },
        );

        let auth_tip = if !current_registry.username.is_empty() {
            rust_i18n::t!(
                "container.push.auth_user_hint",
                user = current_registry.username
            )
            .to_string()
        } else {
            rust_i18n::t!("container.push.auth_anonymous_hint").to_string()
        };

        let app_cancel = app.clone();
        let app_confirm = app.clone();

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
                            .child(rust_i18n::t!("container.push.source_image").to_string()),
                    )
                    .child(
                        div()
                            .p_2()
                            .rounded(p.radius.md)
                            .bg(p.muted.opacity(0.4))
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(source_tag),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.push.target_registry").to_string()),
                    )
                    .child(reg_menu),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.push.target_path").to_string()),
                    )
                    .child(Input::new(&target_repo).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.push.target_tag").to_string()),
                    )
                    .child(Input::new(&target_tag).w_full()),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        Icon::new(IconName::Info)
                            .xsmall()
                            .text_color(p.muted_foreground),
                    )
                    .child(div().text_xs().text_color(p.muted_foreground).child(auth_tip)),
            )
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("container-push-cancel")
                            .ghost()
                            .small()
                            .label(rust_i18n::t!("container.push.cancel").to_string())
                            .on_click(move |_, _, cx| {
                                app_cancel.update(cx, |this, cx| {
                                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                        ws.push_dialog = None;
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("container-push-confirm")
                            .primary()
                            .small()
                            .icon(IconName::Upload)
                            .label(rust_i18n::t!("container.push.submit").to_string())
                            .on_click(move |_, _, cx| {
                                app_confirm.update(cx, |this, cx| {
                                    this.submit_container_push_dialog(tab_id, cx);
                                });
                            }),
                    ),
            )
            .into_any_element()
    }
}
