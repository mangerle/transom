//! 运行容器表单与提交。

use super::*;

impl AppShell {
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
            workspace.apply_section(ContainerSection::Containers);
            workspace.notice = Some(rust_i18n::t!("container.run.submitting").to_string());
        }
        cx.notify();
    }
}
