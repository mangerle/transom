//! API 工作区的项目级工具栏与环境、Cookie、Mock 配置面板。

use super::workspace_view::ApiWorkspaceParams;
use super::{ApiAuxiliarySection, ApiWorkspace};
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown, menu_item};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiEnvironmentId, ApiProtocol, CodeTarget};
use transom_ui_icons::{AppIcon, CustomIcon, FileTypeIcon, UiIcon};

/// 协议、环境、项目工具与代码生成工具条。
pub(crate) fn render_workspace_toolbar(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let protocol_options = [
        (ApiProtocol::Http, "HTTP".to_string()),
        (ApiProtocol::GraphQl, "GraphQL".to_string()),
        (
            ApiProtocol::Grpc,
            rust_i18n::t!("api.protocol.grpc_raw").to_string(),
        ),
        (ApiProtocol::WebSocket, "WebSocket".to_string()),
    ];
    let protocol = dropdown(
        "api-request-protocol",
        params.protocol,
        protocol_options.to_vec(),
        |this: &mut ApiWorkspace, protocol, _, cx| this.set_protocol(protocol, cx),
        view.clone(),
    );
    let environment_options = params
        .project
        .environments
        .iter()
        .map(|environment| (Some(environment.id), environment.name.clone()))
        .collect::<Vec<_>>();
    let environment = dropdown(
        "api-request-environment",
        params.environment_id,
        environment_options,
        |this: &mut ApiWorkspace, environment_id: Option<ApiEnvironmentId>, window, cx| {
            if let Some(environment_id) = environment_id {
                this.set_environment(environment_id, window, cx);
            }
        },
        view.clone(),
    );

    let tools = h_flex()
        .gap(rems(space::XXS))
        .items_center()
        .child(project_tool_button(
            "api-project-environment",
            AppIcon::Ui(UiIcon::DEPENDENCIES),
            rust_i18n::t!("api.project.environment").to_string(),
            ApiAuxiliarySection::Environment,
            params.auxiliary_section,
            view.clone(),
        ))
        .child(project_tool_button(
            "api-project-cookies",
            AppIcon::Custom(CustomIcon::COOKIE),
            rust_i18n::t!("api.project.cookies").to_string(),
            ApiAuxiliarySection::Cookies,
            params.auxiliary_section,
            view.clone(),
        ))
        .child(project_tool_button(
            "api-project-mock",
            AppIcon::Ui(UiIcon::WEB_SERVER),
            rust_i18n::t!("api.project.mock").to_string(),
            ApiAuxiliarySection::Mock,
            params.auxiliary_section,
            view.clone(),
        ))
        .child(render_codegen_menu(params));

    h_flex()
        .flex_wrap()
        .gap(rems(space::XS))
        .items_center()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(protocol)
        .child(environment)
        .child(div().flex_1().min_w_0())
        .child(tools)
        .into_any_element()
}

fn project_tool_button(
    id: &'static str,
    icon: AppIcon,
    label: String,
    section: ApiAuxiliarySection,
    current: Option<ApiAuxiliarySection>,
    view: Entity<ApiWorkspace>,
) -> AnyElement {
    IconButton::new(id, icon, label)
        .size(IconButtonSize::Compact)
        .selected(current == Some(section))
        .render()
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| this.toggle_auxiliary(section, cx));
        })
        .into_any_element()
}

fn render_codegen_menu(params: &ApiWorkspaceParams<'_>) -> AnyElement {
    let view = params.view.clone();
    action_button(
        "api-codegen",
        AppIcon::Ui(UiIcon::LANGUAGE),
        rust_i18n::t!("api.request.codegen").to_string(),
    )
    .small()
    .dropdown_caret(true)
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let targets = [
            (CodeTarget::Curl, "cURL", FileTypeIcon::SHELL),
            (
                CodeTarget::JavaScriptFetch,
                "JavaScript fetch",
                FileTypeIcon::JAVASCRIPT,
            ),
            (
                CodeTarget::PythonRequests,
                "Python requests",
                FileTypeIcon::PYTHON,
            ),
            (CodeTarget::RustReqwest, "Rust reqwest", FileTypeIcon::RUST),
        ];
        targets
            .into_iter()
            .fold(menu, |menu, (target, label, icon)| {
                let view = view.clone();
                menu.item(menu_item(
                    label,
                    AppIcon::FileType(icon),
                    move |_, _, cx| {
                        view.update(cx, |this, cx| this.copy_code(target, cx));
                    },
                ))
            })
    })
    .into_any_element()
}

/// 项目级配置面板；只渲染环境、Cookie 或 Mock 中当前选中的一项。
pub(crate) fn render_auxiliary_panel(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let Some(section) = params.auxiliary_section else {
        return div().into_any_element();
    };
    let (title, body) = match section {
        ApiAuxiliarySection::Environment => (
            rust_i18n::t!("api.request.aux_environment").to_string(),
            editor("api-env-editor", params.environment_editor),
        ),
        ApiAuxiliarySection::Cookies => (
            rust_i18n::t!("api.request.aux_cookies").to_string(),
            editor("api-cookie-editor", params.cookies_editor),
        ),
        ApiAuxiliarySection::Mock => {
            let view = params.view.clone();
            let title = rust_i18n::t!("api.request.aux_mock").to_string();
            let body = v_flex()
                .gap(rems(space::XS))
                .child(editor("api-mock-editor", params.mock_editor))
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .child(
                            action_button(
                                "api-mock-start",
                                AppIcon::Ui(UiIcon::RUN),
                                rust_i18n::t!("api.request.mock_start").to_string(),
                            )
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| this.start_mock(window, cx));
                                }
                            }),
                        )
                        .child(
                            action_button(
                                "api-mock-stop",
                                AppIcon::Ui(UiIcon::STOP),
                                rust_i18n::t!("api.request.mock_stop").to_string(),
                            )
                            .disabled(params.mock_port.is_none())
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.stop_mock(cx));
                            }),
                        )
                        .when_some(params.mock_port, |row, port| {
                            row.child(
                                div()
                                    .text_color(p.muted_foreground)
                                    .child(format!("http://127.0.0.1:{port}")),
                            )
                        }),
                )
                .into_any_element();
            (title, body)
        }
    };
    let view = params.view.clone();
    v_flex()
        .id("api-project-panel")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .child(
            h_flex()
                .items_center()
                .gap(rems(space::SM))
                .child(div().flex_1().text_color(p.foreground).child(title))
                .child(
                    IconButton::new(
                        "api-project-panel-close",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("common.close").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.auxiliary_section = None;
                            cx.notify();
                        });
                    }),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .rounded(p.radius.md)
                .border_1()
                .border_color(p.divider(p.border))
                .child(body),
        )
        .into_any_element()
}

fn editor(id: &'static str, state: &Entity<TextareaState>) -> AnyElement {
    v_flex()
        .id(id)
        .size_full()
        .min_h_0()
        .child(Textarea::new(state).size_full())
        .into_any_element()
}
