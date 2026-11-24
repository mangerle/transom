//! API 项目工作区视图：接口树、请求编辑器和响应面板。

use super::workspace_advanced_view::{render_auxiliary_panel, render_protocol_bar};
use super::{ApiAuxiliarySection, ApiWorkspace};
use crate::divider::{h_resizable, v_resizable};
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, size, space, typography};
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_api::{
    ApiAssertionResult, ApiEnvironmentId, ApiMethod, ApiProject, ApiProtocol, ApiRequestId,
    ApiResponseSnapshot,
};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// API 工作区渲染参数；所有跨实体数据由外壳在本帧读取后传入。
pub(crate) struct ApiWorkspaceParams<'a> {
    pub(crate) view: &'a Entity<ApiWorkspace>,
    pub(crate) project: &'a ApiProject,
    pub(crate) selected_request: Option<ApiRequestId>,
    pub(crate) method: ApiMethod,
    pub(crate) protocol: ApiProtocol,
    pub(crate) environment_id: Option<ApiEnvironmentId>,
    pub(crate) auxiliary_section: Option<ApiAuxiliarySection>,
    pub(crate) ws_connected: bool,
    pub(crate) mock_port: Option<u16>,
    pub(crate) notice: Option<&'a str>,
    pub(crate) response: Option<&'a ApiResponseSnapshot>,
    pub(crate) sending: bool,
    pub(crate) error: Option<&'a str>,
    pub(crate) request_name: &'a Entity<InputState>,
    pub(crate) request_url: &'a Entity<InputState>,
    pub(crate) request_headers: &'a Entity<TextareaState>,
    pub(crate) request_body: &'a Entity<TextareaState>,
    pub(crate) grpc_service: &'a Entity<InputState>,
    pub(crate) grpc_method: &'a Entity<InputState>,
    pub(crate) graphql_variables: &'a Entity<TextareaState>,
    pub(crate) auth: &'a Entity<TextareaState>,
    pub(crate) assertions: &'a Entity<TextareaState>,
    pub(crate) environment_editor: &'a Entity<TextareaState>,
    pub(crate) cookies_editor: &'a Entity<TextareaState>,
    pub(crate) mock_editor: &'a Entity<TextareaState>,
    pub(crate) history_editor: &'a Entity<TextareaState>,
    pub(crate) websocket_input: &'a Entity<TextareaState>,
    pub(crate) assertion_results: &'a [ApiAssertionResult],
    pub(crate) response_headers: &'a Entity<TextareaState>,
    pub(crate) response_body: &'a Entity<TextareaState>,
    pub(crate) tree_editor_split: &'a Entity<ResizableState>,
    pub(crate) request_response_split: &'a Entity<ResizableState>,
    pub(crate) rem: Pixels,
}

/// 渲染 API 项目工作区。
pub(crate) fn render_api_workspace(params: ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let tree = super::workspace_tree_view::render_tree(&params, p);
    let editor = render_request_editor(&params, p);
    let response = render_response(&params, p);
    let editor_and_response = v_resizable("api-request-response-split")
        .with_state(params.request_response_split)
        .child(
            resizable_panel()
                .size(rem_px(26.0, params.rem))
                .child(editor),
        )
        .child(resizable_panel().child(response));
    let body = h_resizable("api-tree-editor-split")
        .with_state(params.tree_editor_split)
        .child(resizable_panel().size(rem_px(16.0, params.rem)).child(tree))
        .child(resizable_panel().child(editor_and_response));

    v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(body)
        .into_any_element()
}

fn render_request_editor(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view;
    let method_options = ApiMethod::ALL
        .into_iter()
        .map(|method| (method, method.as_str().to_string()))
        .collect();
    let method = dropdown(
        "api-request-method",
        params.method,
        method_options,
        |this: &mut ApiWorkspace, method, _, cx| this.set_method(method, cx),
        view.clone(),
    );
    let view_for_save = view.clone();
    let view_for_send = view.clone();
    let view_for_delete = view.clone();

    let mut content = v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(render_protocol_bar(params, p))
        .child(
            h_flex()
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::XS))
                .items_center()
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(method)
                .child(Input::new(params.request_url).flex_1().min_w_0())
                .child(
                    action_button(
                        "api-request-save",
                        AppIcon::Ui(UiIcon::SAVE),
                        rust_i18n::t!("api.request.save").to_string(),
                    )
                    .on_click(move |_, _, cx| {
                        view_for_save.update(cx, |this, cx| this.save_project(cx));
                    }),
                )
                .child(
                    action_button(
                        "api-request-send",
                        AppIcon::Ui(UiIcon::RUN),
                        rust_i18n::t!("api.request.send").to_string(),
                    )
                    .primary()
                    .disabled(params.sending)
                    .on_click(move |_, window, cx| {
                        view_for_send.update(cx, |this, cx| this.send_request(window, cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-request-delete",
                        AppIcon::Ui(UiIcon::DELETE),
                        rust_i18n::t!("api.request.delete").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .disabled(params.selected_request.is_none())
                    .on_click(move |_, window, cx| {
                        view_for_delete.update(cx, |this, cx| this.delete_selected(window, cx));
                    }),
                ),
        )
        .child(
            h_flex()
                .px(rems(space::SM))
                .pt(rems(space::SM))
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .w(rems(7.0))
                        .text_color(p.muted_foreground)
                        .text_size(rems(typography::CAPTION))
                        .child(rust_i18n::t!("api.request.name").to_string()),
                )
                .child(Input::new(params.request_name).flex_1().min_w_0()),
        );

    if let Some(error) = params.error {
        content = content.child(
            div()
                .mx(rems(space::SM))
                .mt(rems(space::SM))
                .p(rems(space::SM))
                .rounded(p.radius.md)
                .bg(p.surface.control_selected)
                .text_color(p.danger)
                .child(error.to_string()),
        );
    }
    if let Some(notice) = params.notice {
        content = content.child(
            div()
                .mx(rems(space::SM))
                .mt(rems(space::SM))
                .p(rems(space::SM))
                .rounded(p.radius.md)
                .bg(p.surface.control_selected)
                .text_color(p.success)
                .child(notice.to_string()),
        );
    }

    let mut fields = v_flex().w_full();
    if params.protocol == ApiProtocol::GraphQl {
        fields = fields
            .child(field_label(
                rust_i18n::t!("api.request.graphql_variables").to_string(),
                p,
            ))
            .child(
                div()
                    .h(rems(6.0))
                    .px(rems(space::SM))
                    .child(Textarea::new(params.graphql_variables).size_full()),
            );
    }
    if params.protocol == ApiProtocol::Grpc {
        fields = fields.child(
            h_flex()
                .px(rems(space::SM))
                .pt(rems(space::SM))
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .w(rems(7.0))
                        .text_color(p.muted_foreground)
                        .text_size(rems(typography::CAPTION))
                        .child(rust_i18n::t!("api.request.grpc_service").to_string()),
                )
                .child(Input::new(params.grpc_service).flex_1().min_w_0())
                .child(Input::new(params.grpc_method).w(rems(12.0))),
        );
    }
    fields = fields
        .child(field_label(
            rust_i18n::t!("api.request.headers").to_string(),
            p,
        ))
        .child(
            div()
                .h(rems(8.0))
                .px(rems(space::SM))
                .child(Textarea::new(params.request_headers).size_full()),
        )
        .child(field_label(
            rust_i18n::t!("api.request.body").to_string(),
            p,
        ))
        .child(
            div()
                .h(rems(10.0))
                .px(rems(space::SM))
                .pb(rems(space::SM))
                .child(Textarea::new(params.request_body).size_full()),
        );
    let main_area = if params.auxiliary_section.is_some() {
        render_auxiliary_panel(params, p)
    } else {
        div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(fields)
            .into_any_element()
    };
    content.child(main_area).into_any_element()
}

fn field_label(label: String, p: &Palette) -> AnyElement {
    div()
        .px(rems(space::SM))
        .pt(rems(space::SM))
        .pb(rems(space::XS))
        .text_color(p.muted_foreground)
        .text_size(rems(typography::CAPTION))
        .child(label)
        .into_any_element()
}

fn render_response(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let mut body = v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(
            h_flex()
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::SM))
                .items_center()
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(
                    div()
                        .flex_1()
                        .child(rust_i18n::t!("api.response.title").to_string()),
                ),
        );
    if let Some(response) = params.response {
        body = body
            .child(
                h_flex()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(rems(space::SM))
                    .items_center()
                    .text_size(rems(typography::CAPTION))
                    .child(
                        div()
                            .text_color(response_color(response.status, p))
                            .child(format!("{} {}", response.status, response.status_text)),
                    )
                    .child(
                        div().text_color(p.muted_foreground).child(
                            rust_i18n::t!(
                                "api.response.metrics",
                                elapsed = response.elapsed_millis,
                                size = response.body_size
                            )
                            .to_string(),
                        ),
                    )
                    .child(
                        div()
                            .text_color(
                                if params.assertion_results.iter().all(|result| result.passed) {
                                    p.success
                                } else {
                                    p.warning
                                },
                            )
                            .child(
                                rust_i18n::t!(
                                    "api.response.assertions",
                                    passed = params
                                        .assertion_results
                                        .iter()
                                        .filter(|result| result.passed)
                                        .count(),
                                    total = params.assertion_results.len()
                                )
                                .to_string(),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(rems(space::XXS))
                    .children(params.assertion_results.iter().map(|result| {
                        h_flex()
                            .gap(rems(space::XS))
                            .items_center()
                            .text_size(rems(typography::CAPTION))
                            .text_color(if result.passed { p.success } else { p.danger })
                            .child(
                                AppIconElement::new(AppIcon::Ui(if result.passed {
                                    UiIcon::CHECK
                                } else {
                                    UiIcon::ERROR
                                }))
                                .xsmall(),
                            )
                            .child(result.expression.clone())
                            .child(
                                div()
                                    .text_color(p.muted_foreground)
                                    .child(result.detail.clone()),
                            )
                    })),
            )
            .child(
                v_flex()
                    .gap(rems(space::XXS))
                    .px(rems(space::SM))
                    .pt(rems(space::XS))
                    .child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("api.response.headers").to_string()),
                    )
                    .child(
                        Textarea::new(params.response_headers)
                            .readonly(true)
                            .h(rems(8.0)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(rems(space::SM))
                    .pb(rems(space::SM))
                    .child(
                        Textarea::new(params.response_body)
                            .readonly(true)
                            .size_full(),
                    ),
            );
    } else if params.sending {
        body = body.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("api.response.sending").to_string()),
        );
    } else {
        body = body.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("api.response.empty").to_string()),
        );
    }
    body.into_any_element()
}

fn response_color(status: u16, p: &Palette) -> Hsla {
    if status >= 500 {
        p.danger
    } else if status >= 400 {
        p.warning
    } else if (200..300).contains(&status) {
        p.success
    } else {
        p.muted_foreground
    }
}
