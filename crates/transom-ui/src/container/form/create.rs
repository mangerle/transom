//! 拉取镜像 / 新建卷与网络的弹窗。

use super::*;

impl AppShell {
    /// 新建卷/网络/拉取镜像的弹窗体。
    pub(crate) fn render_container_create_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(dialog) = self.state.containers.create_dialog.clone() else {
            return div().into_any_element();
        };
        let tab_id = dialog.tab_id;
        let title = create_button_label(dialog.section)
            .unwrap_or_else(|| rust_i18n::t!("container.create").to_string());
        let name_label = if dialog.section == ContainerSection::Images {
            rust_i18n::t!("container.field.context_path").to_string()
        } else {
            rust_i18n::t!("container.field.name").to_string()
        };
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let workspace = &workspace.read(cx).state;
        let create_input = workspace.create_input.clone();
        let mut content =
            v_flex()
                .gap_3()
                .child(crate::widgets::form_field(name_label, create_input, p));
        if let Some(dockerfile) = dialog.dockerfile.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.dockerfile").to_string(),
                dockerfile,
                p,
            ));
        }
        if let Some(tag) = dialog.tag.clone() {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("container.field.tag").to_string(),
                tag,
                p,
            ));
        }
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
}
