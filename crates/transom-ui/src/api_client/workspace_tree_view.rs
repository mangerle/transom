//! API 工作区接口树视图。

use super::ApiWorkspace;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use crate::widgets::{IconButton, IconButtonSize, list_row, menu_item};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiRequest, ApiRequestId, method_for_request};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(crate) fn render_tree(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view;
    let mut request_rows = v_flex().w_full().gap(rems(space::XXS));
    for folder in &params.project.folders {
        request_rows = request_rows.child(
            h_flex()
                .h(rems(size::TREE_BAND_HEIGHT))
                .px(rems(space::SM))
                .items_center()
                .gap(rems(space::XS))
                .bg(p.list_head)
                .text_color(p.muted_foreground)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).xsmall())
                .child(div().truncate().child(folder.name.clone())),
        );
        for request in params
            .project
            .requests
            .iter()
            .filter(|request| request.folder_id == Some(folder.id))
        {
            request_rows = request_rows.child(render_request_row(
                view,
                request,
                params.selected_request,
                p,
                true,
            ));
        }
    }
    for request in params
        .project
        .requests
        .iter()
        .filter(|request| request.folder_id.is_none())
    {
        request_rows = request_rows.child(render_request_row(
            view,
            request,
            params.selected_request,
            p,
            false,
        ));
    }

    let body = if params.project.requests.is_empty() {
        v_flex()
            .p(rems(space::MD))
            .gap(rems(space::SM))
            .items_center()
            .text_color(p.muted_foreground)
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::WEB_SERVER)).small())
            .child(rust_i18n::t!("api.request.empty_title").to_string())
            .child(rust_i18n::t!("api.request.empty_hint").to_string())
            .into_any_element()
    } else {
        div()
            .p(rems(space::XS))
            .child(request_rows)
            .into_any_element()
    };

    let view_for_new = view.clone();
    v_flex()
        .size_full()
        .min_w_0()
        .overflow_hidden()
        .bg(p.sidebar)
        .border_color(p.divider(p.border))
        .border_r_1()
        .child(
            h_flex()
                .h(rems(size::PANE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .items_center()
                .gap(rems(space::XS))
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(
                    div()
                        .flex_1()
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("api.request.tree_title").to_string()),
                )
                .child(
                    IconButton::new(
                        "api-request-new",
                        AppIcon::Ui(UiIcon::ADD),
                        rust_i18n::t!("api.request.new").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, window, cx| {
                        view_for_new.update(cx, |this, cx| this.new_request(window, cx));
                    }),
                ),
        )
        .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(body))
        .into_any_element()
}

fn render_request_row(
    view: &Entity<ApiWorkspace>,
    request: &ApiRequest,
    selected_request: Option<ApiRequestId>,
    p: &Palette,
    nested: bool,
) -> AnyElement {
    let view_for_click = view.clone();
    let view_for_menu = view.clone();
    let request_id = request.id;
    let selected = selected_request == Some(request_id);
    list_row(
        ElementId::Name(format!("api-request-{request_id}").into()),
        selected,
        p,
    )
    .when(nested, |row| row.ml(rems(space::SM)))
    .on_click(move |_, window, cx| {
        view_for_click.update(cx, |this, cx| this.select_request(request_id, window, cx));
    })
    .context_menu(move |menu, _window, _cx| {
        let view = view_for_menu.clone();
        menu.item(menu_item(
            rust_i18n::t!("api.request.delete").to_string(),
            AppIcon::Ui(UiIcon::DELETE),
            move |_, window, cx| {
                view.update(cx, |this, cx| {
                    this.select_request(request_id, window, cx);
                    this.delete_selected(window, cx);
                });
            },
        ))
    })
    .child(
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .child(request.display_name().to_string()),
    )
    .child(
        div()
            .text_color(p.primary)
            .text_size(rems(typography::CAPTION))
            .child(method_for_request(request)),
    )
    .into_any_element()
}
