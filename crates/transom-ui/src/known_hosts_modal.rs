//! 已知主机指纹管理弹窗：实体渲染与外壳的事件消费。

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::keys::{KnownHostsEvent, KnownHostsView};
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

impl AppShell {
    /// 已知主机弹窗上报的删除结果：成功写状态栏，失败进应用通知队列。
    pub(crate) fn on_known_hosts_event(&mut self, event: &KnownHostsEvent, cx: &mut Context<Self>) {
        match event {
            KnownHostsEvent::Removed { error: Some(error) } => {
                self.state.set_error(localized_error(error))
            }
            KnownHostsEvent::Removed { error: None } => {
                self.state.status = Some(rust_i18n::t!("known_hosts.deleted").to_string());
            }
        }
        cx.notify();
    }
}

impl KnownHostsView {
    /// 弹窗主体：记录列表、搜索框与关闭按钮。
    ///
    /// 渲染期实体已被租借，因此只读自身状态；按钮回调经 `cx.listener` 回到
    /// 本实体，不再经外壳转发。
    pub(crate) fn render_body(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let records = self.records().to_vec();
        let search = self.search().clone();
        let search_query = search.read(cx).value().trim().to_lowercase();
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
                rows.push(known_host_row(index, &record, cx, p));
            }
        }

        let content = v_flex().gap_3().child(Input::new(&search).w_full()).child(
            v_flex()
                .id(ElementId::Name("known-hosts-list".into()))
                .max_h(px(380.0))
                .overflow_y_scroll()
                .children(rows),
        );

        let footer = Button::new("close-kh-btn")
            .ghost()
            .label(rust_i18n::t!("app.close").to_string())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)));

        crate::widgets::dialog_body(
            "known-hosts-modal",
            rust_i18n::t!("known_hosts.title").to_string(),
            None,
            content,
            Some(footer),
        )
    }
}

/// 单条已知主机记录：主机/端口、算法、指纹与删除按钮。
fn known_host_row(
    index: usize,
    record: &transom_core::HostKeyRecord,
    cx: &mut Context<KnownHostsView>,
    p: &Palette,
) -> AnyElement {
    let host = record.host.clone();
    let port = record.port;
    h_flex()
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
                                .child(record.algorithm.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(record.fingerprint.clone()),
                ),
        )
        .child(
            crate::widgets::IconButton::new(
                ElementId::Name(format!("del-kh-{index}").into()),
                AppIcon::Ui(UiIcon::DELETE),
                rust_i18n::t!("common.delete").to_string(),
            )
            .size(crate::widgets::IconButtonSize::Regular)
            .render()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.remove(&host, port, cx);
            })),
        )
        .into_any_element()
}
