//! 容器引擎连接表单的基础信息卡、分组选择与底部操作区。

use super::*;

impl AppShell {
    pub(super) fn render_engine_form_basic_card(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
        h_flex()
            .gap_3()
            .child(div().flex_1().child(crate::widgets::form_field(
                rust_i18n::t!("container.form.name").to_string(),
                form.name.clone(),
                p,
            )))
            .child(
                div()
                    .flex_1()
                    .child(self.render_container_form_folder(app, form, p)),
            )
            .into_any_element()
    }

    fn render_container_form_folder(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
        let folders = self.state.catalog.engine_folders.clone();
        let selected = form.folder_id;
        let label = selected
            .and_then(|id| self.state.catalog.engine_folder(id))
            .map(|folder| folder.name.clone())
            .unwrap_or_else(|| rust_i18n::t!("folder.root_level").to_string());
        let app = app.clone();
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.group").to_string()),
            )
            .child(
                Button::new("container-form-folder")
                    .small()
                    .icon(IconName::Folder)
                    .label(label)
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu;
                        let root_app = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!("folder.root_level").to_string())
                                .checked(selected.is_none())
                                .on_click(move |_, _, cx| {
                                    root_app.update(cx, |this, cx| {
                                        if let Some(form) = this.state.containers.form.as_mut() {
                                            form.folder_id = None;
                                            form.test_status = None;
                                        }
                                        cx.notify();
                                    });
                                }),
                        );
                        for folder in &folders {
                            let folder_id = folder.id;
                            let item_app = app.clone();
                            menu = menu.item(
                                PopupMenuItem::new(folder.name.clone())
                                    .checked(selected == Some(folder_id))
                                    .on_click(move |_, _, cx| {
                                        item_app.update(cx, |this, cx| {
                                            if let Some(form) = this.state.containers.form.as_mut()
                                            {
                                                form.folder_id = Some(folder_id);
                                                form.test_status = None;
                                            }
                                            cx.notify();
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .into_any_element()
    }

    // -------------------------------------------------------------------------
    // 外观卡
    // -------------------------------------------------------------------------

    pub(super) fn render_engine_form_footer(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
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

        let mut footer_block = v_flex().gap_2().items_end();
        if let Some(error) = &form.test_status
            && !form.test_ok
        {
            footer_block =
                footer_block.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer_block = footer_block.child(footer);
        footer_block.into_any_element()
    }
}
