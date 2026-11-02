//! 引擎分组新建/重命名表单。

use super::*;

impl AppShell {
    /// 引擎分组的新建/重命名弹窗体。
    pub(crate) fn render_engine_folder_form_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state.containers.engine_folder_form.as_ref() else {
            return div().into_any_element();
        };
        let editing = form.editing.is_some();
        let parent = form.parent;
        let input = form.input.clone();
        let error = form.error.clone();
        let title = if editing {
            rust_i18n::t!("container.sidebar.folder_edit_title").to_string()
        } else {
            rust_i18n::t!("container.sidebar.folder_new_title").to_string()
        };
        let description = parent
            .and_then(|id| self.state.catalog.engine_folder_path(id))
            .map(|path| rust_i18n::t!("container.sidebar.folder_parent", path = path).to_string());
        let mut content = v_flex().gap_3().child(crate::widgets::form_field(
            rust_i18n::t!("container.sidebar.folder_name").to_string(),
            input,
            p,
        ));
        if let Some(error) = error {
            content = content.child(div().text_xs().text_color(p.danger).child(error));
        }
        let footer = h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("container-engine-folder-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.dismiss_engine_folder_form(cx),
                    )),
            )
            .child(
                Button::new("container-engine-folder-save")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, window, cx| {
                            this.submit_engine_folder_form(window, cx);
                        },
                    )),
            );
        crate::widgets::dialog_body(
            "container-engine-folder-body",
            title,
            description,
            content,
            Some(footer),
        )
    }
}
