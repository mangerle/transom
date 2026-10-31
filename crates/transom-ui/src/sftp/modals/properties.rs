//! SFTP 弹窗视图。

use super::common::*;

impl AppShell {
    pub(crate) fn render_sftp_properties_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.state.sftp_modals.properties.as_ref() else {
            return div().into_any_element();
        };
        let scope = modal.scope.clone();
        let properties = modal.properties.clone();
        let name = properties.name.clone();
        let path = properties.path.clone();

        let mut rows: Vec<AnyElement> = Vec::with_capacity(9);
        rows.push(property_row(
            rust_i18n::t!("sftp.properties.label_name").as_ref(),
            name.clone(),
            p,
        ));
        rows.push(property_row(
            rust_i18n::t!("sftp.properties.label_kind").as_ref(),
            properties.kind.clone(),
            p,
        ));
        if let Some(size) = properties.size {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_size").as_ref(),
                rust_i18n::t!(
                    "sftp.properties.size_value",
                    size = format_size(size),
                    bytes = size
                )
                .to_string(),
                p,
            ));
        }
        if let Some(permissions) = properties.permissions {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_permissions").as_ref(),
                format!(
                    "{} ({})",
                    format_rwx_permissions(permissions),
                    format_octal_permissions(permissions)
                ),
                p,
            ));
        }
        if let Some(owner) = properties.owner.clone() {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_owner").as_ref(),
                owner,
                p,
            ));
        }
        if let Some(group) = properties.group.clone() {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_group").as_ref(),
                group,
                p,
            ));
        }
        if let Some(modified) = properties.modified {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_modified").as_ref(),
                format_unix_time(modified),
                p,
            ));
        }
        if let Some(accessed) = properties.accessed {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_accessed").as_ref(),
                format_unix_time(accessed),
                p,
            ));
        }
        if let Some(link_target) = properties.link_target.clone() {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_link_target").as_ref(),
                link_target,
                p,
            ));
        }

        let content = v_flex().gap_1().children(rows);

        let footer = h_flex().gap_2().child(
            Button::new("close-sftp-properties")
                .ghost()
                .icon(IconName::Close)
                .label(rust_i18n::t!("app.close").to_string())
                .on_click(crate::dialog_host::on_event(
                    app,
                    |this, _: &ClickEvent, window, cx| {
                        this.state.sftp_modals.properties = None;
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    },
                )),
        );

        let scope_label = if scope == "本地" {
            rust_i18n::t!("sftp.local_title").to_string()
        } else {
            rust_i18n::t!("sftp.modals.scope_remote").to_string()
        };

        crate::widgets::dialog_body(
            "sftp-properties-modal",
            rust_i18n::t!("sftp.properties.title", name = name.as_str()).to_string(),
            Some(
                rust_i18n::t!(
                    "sftp.properties.subtitle",
                    scope = scope_label,
                    path = path.as_str()
                )
                .to_string(),
            ),
            content,
            Some(footer),
        )
    }
}

/// 渲染属性弹窗中的单行「标签 - 值」条目
fn property_row(label: &str, value: String, p: &Palette) -> AnyElement {
    h_flex()
        .gap_3()
        .py_1p5()
        .px_3()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.3))
        .items_start()
        .child(
            div()
                .w(px(110.0))
                .text_xs()
                .text_color(p.muted_foreground)
                .child(label.to_string()),
        )
        .child(div().flex_1().text_sm().child(value))
        .into_any_element()
}
