//! 空状态与引擎发现候选。

use super::*;
use crate::container::types::ContainerDiscovery;
use crate::widgets::{IconButton, IconButtonSize, action_button};
use gpui_kit::component::button::ButtonVariants;
use transom_core::Connection;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 渲染空列表提示与两个入口按钮。
pub(crate) fn render_engine_empty(
    view: &Entity<ContainerSidebarView>,
    query: &str,
    p: &Palette,
) -> AnyElement {
    if !query.is_empty() {
        return div()
            .p_2()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.sidebar.no_match").to_string())
            .into_any_element();
    }
    let view_new = view.clone();
    let view_discover = view.clone();
    v_flex()
        .gap_2()
        .p_2()
        .text_sm()
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("container.sidebar.empty").to_string())
        .child(
            h_flex()
                .gap_2()
                .child(
                    action_button(
                        "container-profile-new-empty",
                        UiIcon::ADD,
                        rust_i18n::t!("container.sidebar.new").to_string(),
                    )
                    .primary()
                    .on_click(move |_, _, cx| {
                        view_new.update(cx, |sidebar, cx| {
                            sidebar.request_new_engine(None, cx);
                        });
                    }),
                )
                .child(
                    action_button(
                        "container-discover-empty",
                        DomainIcon::MONITOR,
                        rust_i18n::t!("container.sidebar.discover").to_string(),
                    )
                    .on_click(move |_, _, cx| {
                        view_discover.update(cx, |sidebar, cx| {
                            sidebar.request_discover(cx);
                        });
                    }),
                ),
        )
        .into_any_element()
}

/// 发现结果面板：列出可用引擎并给「添加并打开」，同时把不可用原因摊开。
///
/// 设计原理（Why）：候选与失败原因来自目录快照与侧栏状态；SSH 会话联动列表
/// 也从目录快照生成，面板不反查外壳。
pub(crate) fn render_container_discovery(
    view: &Entity<ContainerSidebarView>,
    _state: &ContainerSidebarViewState,
    discovery: &ContainerDiscovery,
    catalog: &SessionCatalog,
    p: &Palette,
) -> AnyElement {
    let mut body = v_flex()
        .gap_1()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.surface.active);
    let view_dismiss = view.clone();
    body = body.child(
        h_flex()
            .gap_1()
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        AppIconElement::new(AppIcon::Domain(DomainIcon::MONITOR))
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("container.discovery.title").to_string()),
                    ),
            )
            .child(
                IconButton::new(
                    "container-discovery-dismiss",
                    UiIcon::CLOSE,
                    "Dismiss container discovery",
                )
                .size(IconButtonSize::RowAction)
                .show_tooltip(false)
                .render()
                .on_click(move |_, _, cx| {
                    view_dismiss.update(cx, |sidebar, cx| {
                        sidebar.dismiss_discovery(cx);
                    });
                }),
            ),
    );
    if discovery.candidates.is_empty() {
        body = body.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.discovery.none").to_string()),
        );
    }
    for candidate in &discovery.candidates {
        let name = candidate.display_name();
        let endpoint = candidate.engine.describe();
        let view_add = view.clone();
        let candidate_profile = candidate.clone();
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
                    action_button(
                        ElementId::Name(format!("container-discovery-add-{}", candidate.id).into()),
                        UiIcon::ADD,
                        rust_i18n::t!("container.discovery.add").to_string(),
                    )
                    .primary()
                    .on_click(move |_, _, cx| {
                        view_add.update(cx, |sidebar, cx| {
                            sidebar.request_add_discovered(candidate_profile.clone(), cx);
                        });
                    }),
                ),
        );
    }
    for failure in &discovery.failures {
        body = body.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(failure.clone()),
        );
    }

    // 联动发现：扫描已保存的 SSH 会话供一键直连 Docker。
    let ssh_sessions: Vec<_> = catalog
        .sessions
        .iter()
        .filter(|session| matches!(session.connection, Connection::Ssh(_)))
        .cloned()
        .collect();

    if !ssh_sessions.is_empty() {
        body = body.child(
            div()
                .mt_2()
                .pt_1()
                .border_t_1()
                .border_color(p.stroke.medium)
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.discovery.ssh_sessions_title").to_string()),
        );
        for session in ssh_sessions {
            let session_id = session.id;
            let session_name = session.name.clone();
            let host_desc = match &session.connection {
                Connection::Ssh(ssh) => format!("{}:{}", ssh.host, ssh.port),
                _ => String::new(),
            };
            let view_session = view.clone();
            body = body.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_xs().truncate().child(session_name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .truncate()
                                    .child(host_desc),
                            ),
                    )
                    .child(
                        action_button(
                            ElementId::Name(
                                format!("container-discovery-ssh-{}", session_id).into(),
                            ),
                            DomainIcon::CONTAINER,
                            rust_i18n::t!("container.discovery.connect_docker").to_string(),
                        )
                        .on_click(move |_, _, cx| {
                            view_session.update(cx, |sidebar, cx| {
                                sidebar.request_open_session(session_id, cx);
                            });
                        }),
                    ),
            );
        }
    }

    body.into_any_element()
}
