//! 容器引擎连接类型、端点附加字段与平台条件选项。

use super::*;

/// 端点选项分组：本地与远端两大类，每组内列出可用档位。
const LOCAL_CHOICES: &[EndpointChoice] = &[
    EndpointChoice::Auto,
    EndpointChoice::Unix,
    EndpointChoice::NamedPipe,
    EndpointChoice::Wsl,
    EndpointChoice::Wslc,
];
const REMOTE_CHOICES: &[EndpointChoice] = &[
    EndpointChoice::Tcp,
    EndpointChoice::Ssh,
    EndpointChoice::Cli,
];

impl AppShell {
    pub(super) fn render_engine_form_connection_card(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
        let available = EndpointChoice::available();

        // 引擎种类行
        let kind_row = h_flex()
            .gap_2()
            .child(
                Button::new("container-kind-docker")
                    .small()
                    .label("Docker")
                    .selected(form.kind == ContainerEngineKind::Docker)
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.state.containers.form.as_mut() {
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
                            if let Some(form) = this.state.containers.form.as_mut() {
                                form.kind = ContainerEngineKind::Podman;
                            }
                            cx.notify();
                        },
                    )),
            );

        // 本地端点选项按钮
        let local_buttons = h_flex().gap_1p5().flex_wrap().children(
            LOCAL_CHOICES
                .iter()
                .filter(|&&c| available.contains(&c))
                .map(|&choice| {
                    let selected = form.endpoint_choice == choice;
                    Self::endpoint_choice_button(choice, selected, app, p)
                }),
        );

        // 远端端点选项按钮
        let remote_buttons = h_flex().gap_1p5().flex_wrap().children(
            REMOTE_CHOICES
                .iter()
                .filter(|&&c| available.contains(&c))
                .map(|&choice| {
                    let selected = form.endpoint_choice == choice;
                    Self::endpoint_choice_button(choice, selected, app, p)
                }),
        );

        v_flex()
            .gap_3()
            // 引擎种类
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.form.kind").to_string()),
                    )
                    .child(kind_row),
            )
            // 本地连接
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.form.local_endpoint").to_string()),
                    )
                    .child(local_buttons),
            )
            // 远端连接
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.form.remote_endpoint").to_string()),
                    )
                    .child(remote_buttons),
            )
            .into_any_element()
    }

    /// 单个端点选项按钮；切换时重置测试状态并更新占位提示。
    fn endpoint_choice_button(
        choice: EndpointChoice,
        selected: bool,
        app: &Entity<AppShell>,
        _p: &Palette,
    ) -> AnyElement {
        Button::new(ElementId::Name(
            format!("container-endpoint-{choice:?}").into(),
        ))
        .xsmall()
        .label(rust_i18n::t!(choice.label_key()).to_string())
        .selected(selected)
        .on_click(crate::dialog_host::on_event(
            app,
            move |this, _: &ClickEvent, window, cx| {
                if let Some(form) = this.state.containers.form.as_mut() {
                    form.endpoint_choice = choice;
                    form.test_status = None;
                    form.endpoint_value.update(cx, |input, cx| {
                        input.set_placeholder(
                            rust_i18n::t!(choice.placeholder_key()).to_string(),
                            window,
                            cx,
                        );
                    });
                }
                cx.notify();
            },
        ))
        .into_any_element()
    }

    // -------------------------------------------------------------------------
    // 动态扩展字段
    // -------------------------------------------------------------------------

    /// 根据当前端点档位返回附加字段卡片；无附加字段时返回 `None`。
    pub(super) fn render_engine_form_extra_fields(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> Option<AnyElement> {
        match form.endpoint_choice {
            EndpointChoice::Auto | EndpointChoice::Wslc => None,

            EndpointChoice::Unix | EndpointChoice::NamedPipe | EndpointChoice::Wsl => {
                // 只有主输入框（路径/发行版名），直接显示
                Some(
                    crate::widgets::form_field(
                        rust_i18n::t!(form.endpoint_choice.field_label_key()?).to_string(),
                        form.endpoint_value.clone(),
                        p,
                    )
                    .into_any_element(),
                )
            }

            EndpointChoice::Tcp => Some(self.render_engine_form_tcp_fields(app, form, p)),

            EndpointChoice::Ssh => Some(self.render_engine_form_ssh_fields(app, form, p)),

            EndpointChoice::Cli => Some(
                v_flex()
                    .gap_3()
                    .child(crate::widgets::form_field(
                        rust_i18n::t!("container.form.cli_program").to_string(),
                        form.endpoint_value.clone(),
                        p,
                    ))
                    .child(crate::widgets::form_field(
                        rust_i18n::t!("container.form.cli_args").to_string(),
                        form.cli_args.clone(),
                        p,
                    ))
                    .into_any_element(),
            ),
        }
    }

    /// TCP 端点字段：URL 输入 + 可选 TLS 证书配置区块。
    fn render_engine_form_tcp_fields(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
        let tls_enabled = form.tls.enabled;

        let mut block = v_flex()
            .gap_3()
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.form.endpoint_url").to_string(),
                form.endpoint_value.clone(),
                p,
            ))
            // TLS 开关
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("container-form-tls-enable")
                            .small()
                            .checked(tls_enabled)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, &checked, _, cx| {
                                    if let Some(form) = this.state.containers.form.as_mut() {
                                        form.tls.enabled = checked;
                                        form.test_status = None;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("container.form.tls_enable").to_string()),
                    ),
            );

        // TLS 证书字段（仅启用时显示）
        if tls_enabled {
            block = block
                .child(crate::widgets::form_field(
                    rust_i18n::t!("container.form.tls_ca").to_string(),
                    form.tls.ca_cert.clone(),
                    p,
                ))
                .child(
                    h_flex()
                        .gap_3()
                        .child(div().flex_1().child(crate::widgets::form_field(
                            rust_i18n::t!("container.form.tls_cert").to_string(),
                            form.tls.client_cert.clone(),
                            p,
                        )))
                        .child(div().flex_1().child(crate::widgets::form_field(
                            rust_i18n::t!("container.form.tls_key").to_string(),
                            form.tls.client_key.clone(),
                            p,
                        ))),
                );
        }

        block.into_any_element()
    }

    /// SSH 端点字段：主机 + 端口/用户（同排）+ 私钥路径。
    fn render_engine_form_ssh_fields(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
        let use_sudo = form.ssh.use_sudo;
        let mut fields = v_flex()
            .gap_3()
            // 主机（单独一行，最常填的字段）
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.form.endpoint_ssh_host").to_string(),
                form.endpoint_value.clone(),
                p,
            ))
            // 端口 + 用户（同排，各占一半）
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.form.ssh_port").to_string(),
                        form.ssh.port.clone(),
                        p,
                    )))
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("container.form.ssh_user").to_string(),
                        form.ssh.user.clone(),
                        p,
                    ))),
            )
            // 登录密码（单独一行）
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.form.ssh_password").to_string(),
                form.ssh.password.clone(),
                p,
            ))
            // 私钥文件（单独一行）
            .child(crate::widgets::form_field(
                rust_i18n::t!("container.form.ssh_identity").to_string(),
                form.ssh.identity_file.clone(),
                p,
            ))
            // sudo 提权开关
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("container-form-ssh-sudo")
                            .small()
                            .checked(use_sudo)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, &checked, _, cx| {
                                    if let Some(form) = this.state.containers.form.as_mut() {
                                        form.ssh.use_sudo = checked;
                                        form.test_status = None;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("container.form.ssh_use_sudo").to_string()),
                    ),
            );

        if use_sudo {
            fields = fields.child(crate::widgets::form_field(
                rust_i18n::t!("container.form.ssh_sudo_password").to_string(),
                form.ssh.sudo_password.clone(),
                p,
            ));
        }

        fields.into_any_element()
    }

    // -------------------------------------------------------------------------
    // 页脚
    // -------------------------------------------------------------------------
}
