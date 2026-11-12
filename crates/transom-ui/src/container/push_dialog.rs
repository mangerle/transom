//! 镜像推送到远端仓库（Docker Hub / Harbor）弹窗界面。
//!
//! 设计原理（Why）：
//! - 将推送镜像与已配置的 `ContainerRegistry` 凭证无缝联动，用户只需下拉选择目标源；
//! - 自动根据所选仓库域名与源镜像 Tag 智能补全目标 Tag 前缀与版本；
//! - 自动注入 Base64 AuthConfig 凭证头部，彻底解决私有仓库 401/403 鉴权失败问题。
//!
//! 动作归属（Why）：打开、提交与关闭只读写本标签页状态并投递命令；仓库配置
//! 从全局设置读取，不依赖外壳字段，因此全部落在 [`ContainerWorkspace`] 上。

use super::list::dropdown_trigger;
use super::*;
use crate::palette::Palette;
use crate::widgets::{DialogConfirmKind, dialog_cancel_button, dialog_confirm_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ContainerCommand, ContainerRegistry, ContainerRequestId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl ContainerWorkspace {
    /// 打开推送到远端镜像仓库弹窗。
    pub(crate) fn open_push_dialog(
        &mut self,
        image_id: String,
        source_tag: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let registries = app_settings(cx).container_registries.clone();
        let default_reg = registries
            .iter()
            .find(|registry| registry.is_default)
            .cloned()
            .unwrap_or_else(ContainerRegistry::docker_hub);

        // 拆解源镜像名称与 tag（例如 "nginx:alpine" -> ("nginx", "alpine")）。
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

        let sub1 = cx.subscribe(&target_repo, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_push_dialog(cx);
            }
        });

        let sub2 = cx.subscribe(&target_tag, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_push_dialog(cx);
            }
        });

        self.state.push_dialog = Some(ContainerPushDialog {
            image_id,
            source_tag,
            selected_registry_id: default_reg.id,
            target_repo,
            target_tag,
            error: None,
            _subscriptions: vec![sub1, sub2],
        });
        cx.notify();
    }

    /// 关闭推送弹窗。
    pub(crate) fn cancel_push_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.push_dialog = None;
        cx.notify();
    }

    /// 提交推送到远端请求。
    ///
    /// 目标仓库或标签为空时保持弹窗打开并就地显示错误；成功后关闭弹窗、
    /// 记录进行中的通知并投递「打标签 + 推送」命令。
    pub(crate) fn submit_push_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(push) = self.state.push_dialog.as_ref() else {
            return;
        };
        let image_id = push.image_id.clone();
        let selected_registry_id = push.selected_registry_id.clone();
        let repo = push.target_repo.read(cx).value().trim().to_string();
        let tag = push.target_tag.read(cx).value().trim().to_string();
        let registry = app_settings(cx)
            .container_registries
            .iter()
            .find(|registry| registry.id == selected_registry_id)
            .cloned()
            .unwrap_or_else(ContainerRegistry::docker_hub);

        if repo.is_empty() || tag.is_empty() {
            if let Some(push) = self.state.push_dialog.as_mut() {
                push.error =
                    Some(rust_i18n::t!("container.push.repo_and_tag_required").to_string());
            }
            cx.notify();
            return;
        }

        // 私有仓库补域名；已带域名时不重复拼接。
        let target_repo = registry.qualify_reference(&repo, "");
        let target_full_ref = format!("{target_repo}:{tag}");
        let auth = registry.auth();
        let request_id = ContainerRequestId::new();

        // 打标签与推送放在同一条命令里顺序执行，避免 push 抢在 tag 完成之前。
        self.send_tracked(
            request_id,
            ContainerCommand::PushTaggedImage {
                tab_id: self.tab_id,
                request_id,
                source_image: image_id,
                repo: target_repo,
                tag: Some(tag),
                auth,
            },
        );

        self.state.push_dialog = None;
        self.state.notice = Some(
            rust_i18n::t!("container.push.pushing_notice", reference = target_full_ref).to_string(),
        );
        cx.notify();
    }
}

/// 渲染推送到远端弹窗主体。
///
/// 设计原理（Why）：仓库配置快照由宿主在渲染期传入，弹窗只读状态；下拉切换
/// 仓库、取消与确认都直接 `update` 工作区实体，不再经外壳转发。
pub(crate) fn render_container_push_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    registries: &[ContainerRegistry],
    p: &Palette,
) -> AnyElement {
    let Some(push) = state.push_dialog.as_ref() else {
        return div().into_any_element();
    };

    let source_tag = push.source_tag.clone();
    let selected_reg_id = push.selected_registry_id.clone();
    let target_repo = push.target_repo.clone();
    let target_tag = push.target_tag.clone();
    let error = push.error.clone();

    let current_registry = registries
        .iter()
        .find(|registry| registry.id == selected_reg_id)
        .cloned()
        .unwrap_or_else(ContainerRegistry::docker_hub);

    let registries_snapshot = registries.to_vec();
    let view_for_menu = view.clone();
    let reg_menu = dropdown_trigger(
        "container-push-registry-select",
        current_registry.name.clone(),
        AppIcon::Ui(UiIcon::CHEVRON_DOWN),
        None,
        move |menu, _, _| {
            let mut menu = menu;
            for registry in &registries_snapshot {
                let view_item = view_for_menu.clone();
                let registry_item = registry.clone();
                let selected = registry.id == selected_reg_id;
                menu = menu.item(
                    PopupMenuItem::new(registry.name.clone())
                        .checked(selected)
                        .on_click(move |_, window, cx| {
                            let registry_item = registry_item.clone();
                            view_item.update(cx, |workspace, cx| {
                                workspace.select_push_registry(&registry_item, window, cx);
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
                    h_flex()
                        .p_3()
                        .gap_2()
                        .items_center()
                        .rounded(p.radius.lg)
                        .bg(p.surface.ghost)
                        .border_1()
                        .border_color(p.border.opacity(0.18))
                        .child(
                            AppIconElement::new(AppIcon::Domain(DomainIcon::CONTAINER))
                                .small()
                                .text_color(p.primary),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(p.mono_font_family.clone())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(source_tag),
                        ),
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
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .rounded(p.radius.md)
                .bg(p.surface.ghost)
                .child(
                    AppIconElement::new(AppIcon::Custom(CustomIcon::INFO))
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(auth_tip),
                ),
        )
        .when_some(error, |this, err| {
            this.child(
                h_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .items_center()
                    .rounded(p.radius.lg)
                    .bg(p.tone.danger_subtle)
                    .text_color(p.danger)
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::WARNING)).small())
                    .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(err)),
            )
        })
        .child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(dialog_cancel_button(
                    "container-push-cancel",
                    rust_i18n::t!("container.push.cancel").to_string(),
                    {
                        let view = view.clone();
                        move |_, _, cx| {
                            view.update(cx, |workspace, cx| {
                                workspace.cancel_push_dialog(cx);
                            });
                        }
                    },
                ))
                .child(dialog_confirm_button(
                    "container-push-confirm",
                    rust_i18n::t!("container.push.submit").to_string(),
                    Some(AppIcon::Ui(UiIcon::UPLOAD)),
                    DialogConfirmKind::Primary,
                    {
                        let view = view.clone();
                        move |_, _, cx| {
                            view.update(cx, |workspace, cx| {
                                workspace.submit_push_dialog(cx);
                            });
                        }
                    },
                )),
        )
        .into_any_element()
}

impl ContainerWorkspace {
    /// 切换推送目标仓库，并按仓库域名补全目标引用前缀。
    ///
    /// 设计原理（Why）：仓库下拉只改本弹窗的选中项与目标输入框；补全规则与
    /// 迁移前一致——Docker Hub 只留镜像名，其余仓库拼上域名前缀。
    fn select_push_registry(
        &mut self,
        registry: &ContainerRegistry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(push) = self.state.push_dialog.as_mut() else {
            return;
        };
        push.selected_registry_id = registry.id.clone();
        let target_repo = push.target_repo.clone();
        let current_value = target_repo.read(cx).value();
        let pure_name = current_value
            .as_ref()
            .rsplit('/')
            .next()
            .unwrap_or(current_value.as_ref());
        let new_value = if registry.id == "docker-hub" {
            pure_name.to_string()
        } else {
            format!("{}/{}", registry.domain(), pure_name)
        };
        target_repo.update(cx, |input, cx| input.set_value(new_value, window, cx));
        cx.notify();
    }
}
