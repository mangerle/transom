//! API 工作区的项目级工具栏与环境、Cookie 配置面板。

use super::ApiAuxiliarySection;
use super::view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown, menu_item};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::*;
use transom_api::CodeTarget;
use transom_ui_icons::{AppIcon, FileTypeIcon, UiIcon};

/// 页签栏右侧的项目动作菜单。
pub(crate) fn render_project_actions_menu(params: &ApiWorkspaceParams<'_>) -> AnyElement {
    let view = params.view.clone();
    Button::new("api-project-actions")
        .ghost()
        .small()
        .label(rust_i18n::t!("common.more").to_string())
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let code_cur = view.clone();
            let code_js = view.clone();
            let code_py = view.clone();
            let code_rs = view.clone();
            menu.item(menu_item(
                "cURL",
                AppIcon::FileType(FileTypeIcon::SHELL),
                move |_, window, cx| {
                    code_cur.update(cx, |this, cx| {
                        this.open_code_preview(CodeTarget::Curl, window, cx)
                    });
                },
            ))
            .item(menu_item(
                "JavaScript fetch",
                AppIcon::FileType(FileTypeIcon::JAVASCRIPT),
                move |_, window, cx| {
                    code_js.update(cx, |this, cx| {
                        this.open_code_preview(CodeTarget::JavaScriptFetch, window, cx)
                    });
                },
            ))
            .item(menu_item(
                "Python requests",
                AppIcon::FileType(FileTypeIcon::PYTHON),
                move |_, window, cx| {
                    code_py.update(cx, |this, cx| {
                        this.open_code_preview(CodeTarget::PythonRequests, window, cx)
                    });
                },
            ))
            .item(menu_item(
                "Rust reqwest",
                AppIcon::FileType(FileTypeIcon::RUST),
                move |_, window, cx| {
                    code_rs.update(cx, |this, cx| {
                        this.open_code_preview(CodeTarget::RustReqwest, window, cx)
                    });
                },
            ))
        })
        .into_any_element()
}

/// 项目级配置面板；只渲染环境或 Cookie 中当前选中的一项。
pub(crate) fn render_auxiliary_panel(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let Some(section) = params.ui.auxiliary_section else {
        return div().into_any_element();
    };
    let (title, body) = match section {
        ApiAuxiliarySection::Code => {
            let view = params.view.clone();
            let copy_view = params.view.clone();
            let target = dropdown(
                "api-codegen-target",
                params.ui.codegen_target,
                vec![
                    (CodeTarget::Curl, CodeTarget::Curl.label().to_string()),
                    (
                        CodeTarget::JavaScriptFetch,
                        CodeTarget::JavaScriptFetch.label().to_string(),
                    ),
                    (
                        CodeTarget::PythonRequests,
                        CodeTarget::PythonRequests.label().to_string(),
                    ),
                    (
                        CodeTarget::RustReqwest,
                        CodeTarget::RustReqwest.label().to_string(),
                    ),
                ],
                |this: &mut super::ApiWorkspace, target, window, cx| {
                    this.set_codegen_target(target, window, cx);
                },
                view,
            );
            let body = v_flex()
                .size_full()
                .min_h_0()
                .gap(rems(space::XS))
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(target)
                        .child(
                            action_button(
                                "api-codegen-copy",
                                AppIcon::Ui(UiIcon::COPY),
                                rust_i18n::t!("api.request.copy_code").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                copy_view.update(cx, |this, cx| this.copy_code_preview(cx));
                            }),
                        ),
                )
                .child(
                    div().flex_1().min_h_0().child(
                        Textarea::new(&params.editors.code_preview)
                            .readonly(true)
                            .size_full(),
                    ),
                )
                .into_any_element();
            (rust_i18n::t!("api.request.code_preview").to_string(), body)
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
                            this.ui.auxiliary_section = None;
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
