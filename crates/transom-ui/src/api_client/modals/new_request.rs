//! 接口树「+」新建与导入弹窗：按两列卡片展示协议新建与导入入口。

use crate::api_client::ApiSidebarEvent;
use crate::api_client::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_api::ApiProtocol;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

/// 渲染新建协议与导入方式弹窗主体。
pub(in crate::api_client) fn render_new_request_modal(
    params: &ApiWorkspaceParams<'_>,
    p: &Palette,
) -> AnyElement {
    let curl_view = params.view.clone();
    let paste_view = params.view.clone();
    let file_view = params.view.clone();
    let sidebar = params.api_sidebar.clone();

    v_flex()
        .id("api-new-request-modal-card")
        .w_full()
        .gap(rems(space::SM))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("api.request.new_section").to_string()),
        )
        .child(
            h_flex()
                .gap(rems(space::SM))
                .child(protocol_card(
                    ProtocolCardSpec {
                        id: "api-new-http",
                        icon: AppIcon::Custom(CustomIcon::API),
                        title: "HTTP",
                        badge: None,
                        protocol: ApiProtocol::Http,
                    },
                    params.view,
                    p,
                ))
                .child(protocol_card(
                    ProtocolCardSpec {
                        id: "api-new-ws",
                        icon: AppIcon::Ui(UiIcon::FETCH),
                        title: "WebSocket",
                        badge: None,
                        protocol: ApiProtocol::WebSocket,
                    },
                    params.view,
                    p,
                )),
        )
        .child(
            h_flex()
                .gap(rems(space::SM))
                .child(protocol_card(
                    ProtocolCardSpec {
                        id: "api-new-grpc",
                        icon: AppIcon::Ui(UiIcon::DATAFLOW),
                        title: "gRPC",
                        badge: Some("Beta"),
                        protocol: ApiProtocol::Grpc,
                    },
                    params.view,
                    p,
                ))
                .child(protocol_card(
                    ProtocolCardSpec {
                        id: "api-new-graphql",
                        icon: AppIcon::Ui(UiIcon::LANGUAGE),
                        title: "GraphQL",
                        badge: Some("Beta"),
                        protocol: ApiProtocol::GraphQl,
                    },
                    params.view,
                    p,
                )),
        )
        .child(
            div()
                .pt(rems(space::XS))
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("api.request.import_section").to_string()),
        )
        .child(
            h_flex()
                .gap(rems(space::SM))
                .child(action_card(
                    "api-import-curl",
                    AppIcon::Ui(UiIcon::UPLOAD),
                    rust_i18n::t!("api.request.curl_import").to_string(),
                    p,
                    move |_, window, cx| {
                        curl_view
                            .update(cx, |this, cx| this.import_from_clipboard(true, window, cx));
                    },
                ))
                .child(action_card(
                    "api-import-paste",
                    AppIcon::Ui(UiIcon::COPY),
                    rust_i18n::t!("api.request.paste_import").to_string(),
                    p,
                    move |_, window, cx| {
                        paste_view
                            .update(cx, |this, cx| this.import_from_clipboard(false, window, cx));
                    },
                )),
        )
        .child(action_card(
            "api-import-file",
            AppIcon::Ui(UiIcon::DOWNLOAD),
            rust_i18n::t!("api.request.other_file_import").to_string(),
            p,
            move |_, _, cx| {
                file_view.update(cx, |this, cx| this.close_new_request_modal(cx));
                sidebar.update(cx, |_, cx| {
                    cx.emit(ApiSidebarEvent::ImportProject);
                    cx.notify();
                });
            },
        ))
        .into_any_element()
}

struct ProtocolCardSpec {
    id: &'static str,
    icon: AppIcon,
    title: &'static str,
    badge: Option<&'static str>,
    protocol: ApiProtocol,
}

fn protocol_card(
    spec: ProtocolCardSpec,
    view: &Entity<crate::api_client::ApiWorkspace>,
    p: &Palette,
) -> AnyElement {
    let view = view.clone();
    let protocol = spec.protocol;
    let mut row = h_flex()
        .id(spec.id)
        .flex_1()
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .px(rems(space::SM))
        .gap(rems(space::SM))
        .items_center()
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.divider(p.border))
        .cursor_pointer()
        .hover(|style| style.bg(p.sidebar_accent.opacity(0.6)))
        .on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
                this.new_request_with_protocol(protocol, window, cx)
            });
        })
        .child(AppIconElement::new(spec.icon).small().text_color(p.primary))
        .child(
            div()
                .flex_1()
                .text_size(rems(typography::BODY))
                .text_color(p.foreground)
                .child(spec.title),
        );
    if let Some(badge) = spec.badge {
        row = row.child(
            div()
                .px(rems(space::XS))
                .py(px(1.0))
                .rounded(p.radius.sm)
                .bg(p.surface.control_selected)
                .text_size(rems(typography::CAPTION))
                .text_color(p.primary)
                .child(badge),
        );
    }
    row.into_any_element()
}

fn action_card(
    id: &'static str,
    icon: AppIcon,
    label: String,
    p: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .flex_1()
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .px(rems(space::SM))
        .gap(rems(space::SM))
        .items_center()
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.divider(p.border))
        .cursor_pointer()
        .hover(|style| style.bg(p.sidebar_accent.opacity(0.6)))
        .on_click(on_click)
        .child(AppIconElement::new(icon).small().text_color(p.warning))
        .child(
            div()
                .flex_1()
                .text_size(rems(typography::BODY))
                .text_color(p.foreground)
                .child(label),
        )
        .into_any_element()
}
