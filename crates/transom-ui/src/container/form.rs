//! 容器表单：保存多个引擎连接，与终端会话表单并列。
//!
//! 设计原理（Why）：引擎连接是独立持久化对象（`SessionCatalog.engines`），
//! 表单只负责编辑端点与名称；“测试连接”走控制器探测，不在 UI 线程建连接。

use crate::app::AppShell;
use crate::container::list::create_button_label;
use crate::container::types::{
    ContainerFormState, ContainerRunForm, ContainerSection, EndpointChoice,
};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::*;
use transom_core::{ContainerCommand, ContainerEngineKind, ContainerRequestId};

mod create;
mod run;

/// 重启策略档位：与引擎允许集合保持一致，留空表示 `no`。
const RUN_RESTART_POLICIES: [&str; 4] = ["no", "on-failure", "always", "unless-stopped"];

impl AppShell {
    /// 打开新建/编辑容器连接表单。
    pub(crate) fn open_container_form(
        &mut self,
        profile: Option<transom_core::ContainerEngineProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 编辑已有连接时清掉暂存分组；从分组入口新建时调用方刚设好目标，不能覆盖。
        if profile.is_some() {
            self.state.containers.form_folder = None;
        }
        let form = ContainerFormState::new(window, cx, profile.as_ref());
        self.state.containers.form = Some(form);
        cx.notify();
    }

    /// 容器连接表单的弹窗体；外观与关闭件由 Dialog 容器负责。
    pub(crate) fn render_container_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.form.as_ref() else {
            return div().into_any_element();
        };
        let title = if form.editing.is_some() {
            rust_i18n::t!("container.form.edit_title").to_string()
        } else {
            rust_i18n::t!("container.form.new_title").to_string()
        };

        let mut content = v_flex()
            .gap_3()
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.form.name").to_string(),
                form.name.clone(),
                p,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.form.kind").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("container-kind-docker")
                                    .small()
                                    .label("Docker")
                                    .selected(form.kind == ContainerEngineKind::Docker)
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        |this, _: &ClickEvent, _, cx| {
                                            if let Some(form) = this.state.containers.form.as_mut()
                                            {
                                                form.kind = ContainerEngineKind::Docker;
                                            }
                                            cx.notify();
                                        },
                                    )),
                            )
                            .child(
                                Button::new("container-kind-podman")
                                    .small()
                                    .label("Podman")
                                    .selected(form.kind == ContainerEngineKind::Podman)
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        |this, _: &ClickEvent, _, cx| {
                                            if let Some(form) = this.state.containers.form.as_mut()
                                            {
                                                form.kind = ContainerEngineKind::Podman;
                                            }
                                            cx.notify();
                                        },
                                    )),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.form.endpoint").to_string()),
                    )
                    .child(h_flex().gap_1p5().flex_wrap().children(
                        EndpointChoice::ALL.into_iter().map(|choice| {
                            let selected = form.endpoint_choice == choice;
                            Button::new(ElementId::Name(
                                format!("container-endpoint-{:?}", choice).into(),
                            ))
                            .xsmall()
                            .label(rust_i18n::t!(choice.label_key()).to_string())
                            .selected(selected)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.state.containers.form.as_mut() {
                                        form.endpoint_choice = choice;
                                        form.test_status = None;
                                    }
                                    cx.notify();
                                },
                            ))
                        }),
                    )),
            );

        if form.endpoint_choice.needs_value() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.form.endpoint_value").to_string(),
                form.endpoint_value.clone(),
                p,
            ));
        }

        if let Some(status) = &form.test_status {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(if form.test_ok { p.primary } else { p.danger })
                    .child(status.clone()),
            );
        }

        let editing = form.editing.is_some();
        let mut footer = h_flex().gap_2().items_center().justify_end().w_full();
        if editing {
            footer = footer.child(
                Button::new("container-form-delete")
                    .danger()
                    .small()
                    .icon(IconName::Trash)
                    .label(rust_i18n::t!("common.delete").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, window, cx| {
                            if let Some(id) = this
                                .state
                                .containers
                                .form
                                .as_ref()
                                .and_then(|form| form.editing)
                            {
                                this.state.containers.form = None;
                                this.state.containers.form_folder = None;
                                this.delete_container_profile(id, window, cx);
                            }
                        },
                    )),
            );
        }
        footer = footer
            .child(
                Button::new("container-form-test")
                    .ghost()
                    .small()
                    .icon(IconName::Gauge)
                    .label(rust_i18n::t!("container.form.test").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.test_container_form(cx),
                    )),
            )
            .child(
                Button::new("container-form-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.state.containers.form = None;
                            this.state.containers.form_folder = None;
                            cx.notify();
                        },
                    )),
            )
            .child(
                Button::new("container-form-save")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.save_container_form(cx),
                    )),
            );

        crate::widgets::dialog_body("container-form-body", title, None, content, Some(footer))
    }

    pub(crate) fn save_container_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.containers.form.as_ref() else {
            return;
        };
        let profile = match form.to_profile(cx) {
            Ok(profile) => profile,
            Err(message) => {
                if let Some(form) = self.state.containers.form.as_mut() {
                    form.test_ok = false;
                    form.test_status = Some(message);
                }
                cx.notify();
                return;
            }
        };
        self.state.containers.form = None;
        self.save_container_profile(profile, cx);
    }

    fn test_container_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.containers.form.as_ref() else {
            return;
        };
        let profile = match form.to_profile(cx) {
            Ok(profile) => profile,
            Err(message) => {
                if let Some(form) = self.state.containers.form.as_mut() {
                    form.test_ok = false;
                    form.test_status = Some(message);
                }
                cx.notify();
                return;
            }
        };
        let request_id = ContainerRequestId::new();
        if let Some(form) = self.state.containers.form.as_mut() {
            form.test_request = Some(request_id);
            form.test_status = Some(rust_i18n::t!("container.form.testing").to_string());
        }
        if !self.state.send_command(ContainerCommand::TestConnection {
            request_id,
            profile,
        }) && let Some(form) = self.state.containers.form.as_mut()
        {
            form.test_request = None;
        }
        cx.notify();
    }
}
