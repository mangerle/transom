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

    /// 打开「运行容器」表单；从镜像行进入时预填镜像引用。
    pub(crate) fn open_container_run_form(
        &mut self,
        tab_id: transom_core::TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.containers.run_form =
            Some(ContainerRunForm::new(tab_id, image_hint, window, cx));
        cx.notify();
    }

    /// 关闭运行表单。
    pub(crate) fn dismiss_container_run(&mut self, cx: &mut Context<Self>) {
        self.state.containers.run_form = None;
        cx.notify();
    }

    /// 运行容器表单的弹窗体。
    pub(crate) fn render_container_run_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return div().into_any_element();
        };
        let tab_id = form.tab_id;
        let restart = form.restart.clone();
        let error = form.error.clone();
        let mut content =
            v_flex()
                .gap_3()
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.image").to_string(),
                    form.image.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.name").to_string(),
                    form.name.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.command").to_string(),
                    form.command.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.ports").to_string(),
                    form.ports.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.binds").to_string(),
                    form.binds.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.env").to_string(),
                    form.env.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.health").to_string(),
                    form.health.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.health_interval").to_string(),
                    form.health_interval.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.memory").to_string(),
                    form.memory.clone(),
                    p,
                ))
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.run.cpus").to_string(),
                    form.cpus.clone(),
                    p,
                ))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.run.restart").to_string()),
                        )
                        .child(h_flex().gap_1().children(RUN_RESTART_POLICIES.iter().map(
                            |policy| {
                                Button::new(ElementId::Name(
                                    format!("container-run-restart-{policy}").into(),
                                ))
                                .xsmall()
                                .ghost()
                                .selected(restart.as_deref() == Some(*policy))
                                .label((*policy).to_string())
                                .on_click(
                                    crate::dialog_host::on_event(
                                        app,
                                        move |this, _: &ClickEvent, _, cx| {
                                            let Some(form) =
                                                this.state.containers.run_form.as_mut()
                                            else {
                                                return;
                                            };
                                            form.restart =
                                                if form.restart.as_deref() == Some(*policy) {
                                                    None
                                                } else {
                                                    Some((*policy).to_string())
                                                };
                                            cx.notify();
                                        },
                                    ),
                                )
                            },
                        ))),
                );
        if let Some(error) = error {
            content = content.child(div().text_xs().text_color(p.danger).child(error));
        }

        let footer = h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("container-run-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.dismiss_container_run(cx),
                    )),
            )
            .child(
                Button::new("container-run-submit")
                    .primary()
                    .icon(IconName::Play)
                    .label(rust_i18n::t!("container.run.submit").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, _, cx| {
                            this.submit_container_run(tab_id, cx);
                        },
                    )),
            );

        // 字段多，让内容区自己滚动；页头页脚由 Dialog 容器固定。
        let body = div()
            .id("container-run-body")
            .max_h(px(440.0))
            .overflow_y_scroll()
            .child(content);
        crate::widgets::dialog_body(
            "container-run-form-body",
            rust_i18n::t!("container.run.title").to_string(),
            None,
            body,
            Some(footer),
        )
    }

    /// 新建卷/网络/拉取镜像的弹窗体。
    pub(crate) fn render_container_create_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(dialog) = self.state.containers.create_dialog.clone() else {
            return div().into_any_element();
        };
        let tab_id = dialog.tab_id;
        let title = create_button_label(dialog.section)
            .unwrap_or_else(|| rust_i18n::t!("container.create").to_string());
        let name_label = if dialog.section == ContainerSection::Images {
            rust_i18n::t!("container.run.image").to_string()
        } else {
            rust_i18n::t!("container.field.name").to_string()
        };
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let create_input = workspace.create_input.clone();
        let mut content =
            v_flex()
                .gap_3()
                .child(crate::widgets::form_field(name_label, create_input, p));
        if let Some(driver) = dialog.driver.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.driver").to_string(),
                driver,
                p,
            ));
        }
        if let Some(subnet) = dialog.subnet.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.subnet").to_string(),
                subnet,
                p,
            ));
        }
        if let Some(gateway) = dialog.gateway.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.gateway").to_string(),
                gateway,
                p,
            ));
        }
        if dialog.section == ContainerSection::Networks {
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("container-create-internal")
                            .small()
                            .checked(dialog.internal)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, &checked, _, cx| {
                                    if let Some(dialog) =
                                        this.state.containers.create_dialog.as_mut()
                                    {
                                        dialog.internal = checked;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("container.create_dialog.internal").to_string()),
                    ),
            );
        }

        let footer = h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("container-create-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.dismiss_container_create(cx),
                    )),
            )
            .child(
                Button::new("container-create-submit")
                    .primary()
                    .icon(if dialog.section == ContainerSection::Images {
                        IconName::Plus
                    } else {
                        IconName::Check
                    })
                    .label(title.clone())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, window, cx| {
                            this.submit_container_create(tab_id, window, cx);
                        },
                    )),
            );
        crate::widgets::dialog_body("container-create-body", title, None, content, Some(footer))
    }

    /// 提交运行：解析并校验 spec 后交给控制器，错误就地展示。
    pub(crate) fn submit_container_run(
        &mut self,
        tab_id: transom_core::TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return;
        };
        let spec = match form.to_spec(cx) {
            Ok(spec) => spec,
            Err(message) => {
                if let Some(form) = self.state.containers.run_form.as_mut() {
                    form.error = Some(message);
                }
                cx.notify();
                return;
            }
        };
        let request_id = ContainerRequestId::new();
        self.state.containers.run_form = None;
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::CreateContainer {
                tab_id,
                request_id,
                spec,
            },
        );
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.notice = Some(rust_i18n::t!("container.run.submitting").to_string());
        }
        cx.notify();
    }
}
