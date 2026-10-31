//! 已知主机指纹管理模态框渲染模块。

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn refresh_known_hosts(&mut self) {
        self.modals.known_hosts.records = self.services.known_hosts.list().unwrap_or_default();
    }

    pub(crate) fn remove_known_host(&mut self, host: &str, port: u16, cx: &mut Context<Self>) {
        if let Err(error) = self.services.known_hosts.remove(host, port) {
            self.set_error(localized_error(&error));
        } else {
            self.status = Some(rust_i18n::t!("known_hosts.deleted").to_string());
        }
        self.refresh_known_hosts();
        cx.notify();
    }

    pub(crate) fn render_known_hosts_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let records = self.modals.known_hosts.records.clone();
        let search_query = self
            .modals
            .known_hosts
            .search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut rows: Vec<AnyElement> = Vec::new();
        let filtered: Vec<_> = records
            .into_iter()
            .filter(|record| {
                if search_query.is_empty() {
                    return true;
                }
                record.host.to_lowercase().contains(&search_query)
                    || record.port.to_string().contains(&search_query)
                    || record.algorithm.to_lowercase().contains(&search_query)
                    || record.fingerprint.to_lowercase().contains(&search_query)
            })
            .collect();

        if filtered.is_empty() {
            rows.push(
                div()
                    .p_4()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("known_hosts.empty").to_string())
                    .into_any_element(),
            );
        } else {
            for (index, record) in filtered.into_iter().enumerate() {
                let host = record.host.clone();
                let port = record.port;
                let row = h_flex()
                    .id(ElementId::Name(format!("known-host-{index}").into()))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .border_color(p.divider(p.border))
                    .divider_edge(DividerEdge::Bottom, p.divider_width)
                    .hover(|style| style.bg(p.muted))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("{}:{}", record.host, record.port)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .px_1()
                                            .rounded(p.radius.sm)
                                            .bg(p.muted)
                                            .text_color(p.muted_foreground)
                                            .child(record.algorithm),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(record.fingerprint),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("del-kh-{index}").into()))
                            .danger()
                            .xsmall()
                            .icon(IconName::Trash)
                            .label(rust_i18n::t!("common.delete").to_string())
                            .on_click(crate::dialog_host::on_event(
                                app,
                                move |this, _: &ClickEvent, _, cx| {
                                    this.remove_known_host(&host, port, cx);
                                },
                            )),
                    )
                    .into_any_element();
                rows.push(row);
            }
        }

        let content = v_flex()
            .gap_3()
            .child(Input::new(&self.modals.known_hosts.search).w_full())
            .child(
                v_flex()
                    .id(ElementId::Name("known-hosts-list".into()))
                    .max_h(px(380.0))
                    .overflow_y_scroll()
                    .children(rows),
            );

        let footer = Button::new("close-kh-btn")
            .ghost()
            .label(rust_i18n::t!("app.close").to_string())
            .on_click(crate::dialog_host::on_event(
                app,
                |this, _: &ClickEvent, _, cx| {
                    this.modals.known_hosts.open = false;
                    cx.notify();
                },
            ));

        Self::dialog_body(
            "known-hosts-modal",
            rust_i18n::t!("known_hosts.title").to_string(),
            None,
            content,
            Some(footer),
        )
    }
}
