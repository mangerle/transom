//! 空状态与引擎发现候选。

use super::*;

impl AppShell {
    pub(crate) fn render_engine_empty(
        &mut self,
        query: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !query.is_empty() {
            return div()
                .p_2()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.sidebar.no_match").to_string())
                .into_any_element();
        }
        v_flex()
            .gap_2()
            .p_2()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.sidebar.empty").to_string())
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("container-profile-new-empty")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("container.sidebar.new").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_container_in_folder(None, window, cx);
                            })),
                    )
                    .child(
                        Button::new("container-discover-empty")
                            .small()
                            .icon(IconName::Radar)
                            .label(rust_i18n::t!("container.sidebar.discover").to_string())
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.discover_engines(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    /// 发现结果面板：列出可用引擎并给「添加并打开」，同时把不可用原因摊开。
    pub(crate) fn render_container_discovery(
        &mut self,
        discovery: ContainerDiscovery,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ContainerDiscovery {
            candidates,
            failures,
        } = discovery;
        let mut body = v_flex()
            .gap_1()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.35));
        body = body.child(
            h_flex()
                .gap_1()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(IconName::Radar).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .child(rust_i18n::t!("container.discovery.title").to_string()),
                        ),
                )
                .child(
                    Button::new("container-discovery-dismiss")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.dismiss_container_discovery(cx);
                        })),
                ),
        );
        if candidates.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.discovery.none").to_string()),
            );
        }
        for candidate in candidates {
            let name = candidate.display_name();
            let endpoint = candidate.engine.describe();
            body = body.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_xs().truncate().child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .truncate()
                                    .child(endpoint),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(
                            format!("container-discovery-add-{}", candidate.id).into(),
                        ))
                        .primary()
                        .xsmall()
                        .icon(IconName::Plus)
                        .label(rust_i18n::t!("container.discovery.add").to_string())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.add_discovered_engine(candidate.clone(), window, cx);
                            },
                        )),
                    ),
            );
        }
        for failure in failures {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(failure),
            );
        }
        body.into_any_element()
    }
}
