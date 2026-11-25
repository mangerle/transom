//! API 项目管理中的内联命名、删除确认和导入预览。

use super::{ApiImportState, ApiSidebarView};
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize, action_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::*;
use transom_api::ApiImportFormat;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(in crate::api_client) fn render_editor(
    view: &Entity<ApiSidebarView>,
    editor: &super::ApiProjectEditor,
    p: &Palette,
) -> AnyElement {
    let submit_view = view.clone();
    let mut row = h_flex()
        .mx(rems(space::SM))
        .my(rems(space::XS))
        .gap(rems(space::XS))
        .items_center()
        .child(Input::new(&editor.input).flex_1().w_full());
    row = row.child(
        IconButton::new(
            "api-project-editor-submit",
            AppIcon::Ui(UiIcon::CHECK),
            rust_i18n::t!("api.project.save").to_string(),
        )
        .size(IconButtonSize::Compact)
        .render()
        .on_click(move |_, _, cx| {
            submit_view.update(cx, |this, cx| this.submit_editor(cx));
        }),
    );
    let cancel_view = view.clone();
    row = row.child(
        IconButton::new(
            "api-project-editor-cancel",
            AppIcon::Ui(UiIcon::CLOSE),
            rust_i18n::t!("api.project.cancel").to_string(),
        )
        .size(IconButtonSize::Compact)
        .render()
        .on_click(move |_, _, cx| {
            cancel_view.update(cx, |this, cx| this.cancel_inline_action(cx));
        }),
    );
    if let Some(error) = &editor.error {
        row = row.child(div().text_color(p.danger).child(error.clone()));
    }
    row.into_any_element()
}

pub(in crate::api_client) fn render_delete_confirm(
    view: &Entity<ApiSidebarView>,
    project_name: &str,
    p: &Palette,
) -> AnyElement {
    let message = rust_i18n::t!("api.project.delete_confirm", name = project_name).to_string();
    let view = view.clone();
    h_flex()
        .mx(rems(space::SM))
        .my(rems(space::XS))
        .gap(rems(space::XS))
        .items_center()
        .child(div().flex_1().text_color(p.foreground).child(message))
        .child(
            action_button(
                "api-project-delete-confirm",
                AppIcon::Ui(UiIcon::DELETE),
                rust_i18n::t!("api.project.delete").to_string(),
            )
            .on_click({
                let view = view.clone();
                move |_, _, cx| view.update(cx, |this, cx| this.confirm_delete(cx))
            }),
        )
        .child(
            Button::new("api-project-delete-cancel")
                .ghost()
                .small()
                .label(rust_i18n::t!("api.project.cancel").to_string())
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| this.cancel_inline_action(cx))
                }),
        )
        .into_any_element()
}

pub(in crate::api_client) fn render_import_state(
    view: &Entity<ApiSidebarView>,
    state: &ApiImportState,
    p: &Palette,
) -> AnyElement {
    match state {
        ApiImportState::Loading { file_name } => h_flex()
            .mx(rems(space::SM))
            .my(rems(space::XS))
            .p(rems(space::SM))
            .gap(rems(space::SM))
            .items_center()
            .rounded(p.radius.md)
            .border_1()
            .border_color(p.divider(p.border))
            .bg(p.surface.control_selected)
            .child(Spinner::new().small().color(p.primary))
            .child(div().flex_1().min_w_0().truncate().child(
                rust_i18n::t!("api.project.import_parsing", file = file_name.as_str()).to_string(),
            ))
            .into_any_element(),
        ApiImportState::Ready(preview) => {
            let request_count = preview
                .projects
                .iter()
                .map(|project| project.request_count())
                .fold(0_usize, usize::saturating_add);
            let visible_names = preview
                .projects
                .iter()
                .take(3)
                .map(|project| project.display_name().to_string())
                .collect::<Vec<_>>()
                .join("、");
            let extra_projects = preview.projects.len().saturating_sub(3);
            let confirm_view = view.clone();
            let cancel_view = view.clone();
            let mut card =
                v_flex()
                    .mx(rems(space::SM))
                    .my(rems(space::XS))
                    .p(rems(space::SM))
                    .gap(rems(space::XS))
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(p.divider(p.border))
                    .bg(p.surface.control_selected)
                    .child(
                        h_flex()
                            .gap(rems(space::XS))
                            .items_center()
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::UPLOAD)).small())
                            .child(div().flex_1().min_w_0().truncate().child(
                                rust_i18n::t!("api.project.import_preview_title").to_string(),
                            )),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(p.muted_foreground)
                            .child(preview.file_name.clone()),
                    )
                    .child(
                        v_flex()
                            .gap(rems(space::XS))
                            .text_color(p.muted_foreground)
                            .child(
                                rust_i18n::t!(
                                    "api.project.import_format",
                                    format = import_format_label(preview.format)
                                )
                                .to_string(),
                            )
                            .child(
                                rust_i18n::t!(
                                    "api.project.import_project_count",
                                    count = preview.projects.len()
                                )
                                .to_string(),
                            )
                            .child(
                                rust_i18n::t!(
                                    "api.project.import_request_count",
                                    count = request_count
                                )
                                .to_string(),
                            ),
                    )
                    .child(div().min_w_0().truncate().child(visible_names));
            if extra_projects > 0 {
                card = card.child(
                    div().text_color(p.muted_foreground).child(
                        rust_i18n::t!("api.project.import_more_projects", count = extra_projects)
                            .to_string(),
                    ),
                );
            }
            card.child(
                h_flex()
                    .gap(rems(space::XS))
                    .child(
                        action_button(
                            "api-project-import-confirm",
                            AppIcon::Ui(UiIcon::CHECK),
                            rust_i18n::t!("api.project.import_confirm").to_string(),
                        )
                        .primary()
                        .disabled(preview.projects.is_empty())
                        .on_click(move |_, _, cx| {
                            confirm_view.update(cx, |this, cx| this.confirm_import(cx));
                        }),
                    )
                    .child(
                        Button::new("api-project-import-cancel")
                            .ghost()
                            .small()
                            .label(rust_i18n::t!("api.project.cancel").to_string())
                            .on_click(move |_, _, cx| {
                                cancel_view.update(cx, |this, cx| this.cancel_import(cx));
                            }),
                    ),
            )
            .into_any_element()
        }
    }
}

fn import_format_label(format: ApiImportFormat) -> String {
    match format {
        ApiImportFormat::Transom => rust_i18n::t!("api.project.import_format_transom").to_string(),
        ApiImportFormat::Curl => rust_i18n::t!("api.project.import_format_curl").to_string(),
        ApiImportFormat::Postman => rust_i18n::t!("api.project.import_format_postman").to_string(),
        ApiImportFormat::OpenApi => rust_i18n::t!("api.project.import_format_openapi").to_string(),
    }
}
