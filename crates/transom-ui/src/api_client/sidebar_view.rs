//! API 项目侧栏视图：搜索、项目行、内联命名和删除确认。

use super::{ApiImportState, ApiSidebarState, ApiSidebarView};
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{
    IconButton, IconButtonSize, SidebarRowState, action_button, empty_state, menu_item,
    sidebar_list_frame, sidebar_row_band,
};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::*;
use transom_api::{ApiExportFormat, ApiImportFormat, ApiProjectId};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 侧栏渲染参数。
pub(crate) struct ApiSidebarParams<'a> {
    pub(crate) view: &'a Entity<ApiSidebarView>,
    pub(crate) state: &'a ApiSidebarState,
    pub(crate) active_project: Option<ApiProjectId>,
    pub(crate) query: &'a str,
}

/// 渲染 API 项目侧栏。
pub(crate) fn render_api_sidebar(params: ApiSidebarParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let state = params.state;
    let projects = filtered_projects(state, params.query);
    let toolbar = h_flex()
        .h(rems(crate::theme::tokens::size::PANE_HEADER_HEIGHT))
        .flex_none()
        .items_center()
        .gap(rems(space::XS))
        .px(rems(space::SM))
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(Input::new(&state.search).w_full().cleanable(false))
        .child(
            IconButton::new(
                "api-project-new",
                AppIcon::Ui(UiIcon::ADD),
                rust_i18n::t!("api.sidebar.new_project").to_string(),
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click({
                let view = view.clone();
                move |_, window, cx| {
                    view.update(cx, |this, cx| this.start_create(window, cx));
                }
            }),
        )
        .child(
            IconButton::new(
                "api-project-import",
                AppIcon::Ui(UiIcon::DOWNLOAD),
                rust_i18n::t!("api.sidebar.import_project").to_string(),
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click({
                let view = view.clone();
                move |_, _, cx| {
                    view.update(cx, |_view, cx| {
                        cx.emit(super::ApiSidebarEvent::ImportProject);
                        cx.notify();
                    });
                }
            }),
        );

    let mut body = v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scrollbar()
        .child(sidebar_list_frame("api-project-list"));

    if let Some(error) = &state.load_error {
        body = body.child(
            div()
                .mx(rems(space::SM))
                .mt(rems(space::XS))
                .p(rems(space::SM))
                .rounded(p.radius.md)
                .bg(p.surface.control_selected)
                .text_color(p.danger)
                .child(error.clone()),
        );
    }

    if let Some(editor) = &state.editor {
        body = body.child(render_editor(&view, editor, p));
    }

    if let Some(project_id) = state.pending_delete
        && let Some(project) = state.registry.project(project_id)
    {
        body = body.child(render_delete_confirm(&view, project.display_name(), p));
    }

    if let Some(import) = &state.pending_import {
        body = body.child(render_import_state(&view, import, p));
    }

    if projects.is_empty() {
        body = body.child(empty_state(
            AppIcon::Ui(UiIcon::WEB_SERVER),
            rust_i18n::t!("api.sidebar.empty_title").to_string(),
            rust_i18n::t!("api.sidebar.empty_hint").to_string(),
            p,
        ));
    } else {
        for project in projects {
            let project_state = if params.active_project == Some(project.id) {
                SidebarRowState::Current
            } else {
                SidebarRowState::Normal
            };
            body = body.child(render_project_row(&view, project, project_state, p));
        }
    }

    v_flex()
        .size_full()
        .min_w_0()
        .overflow_hidden()
        .bg(p.sidebar)
        .child(toolbar)
        .child(body)
        .into_any_element()
}

fn filtered_projects<'a>(
    state: &'a ApiSidebarState,
    query: &str,
) -> Vec<&'a transom_api::ApiProject> {
    let query = query.to_lowercase();
    state
        .registry
        .projects
        .iter()
        .filter(|project| {
            query.is_empty()
                || project.name.to_lowercase().contains(&query)
                || project.base_url.to_lowercase().contains(&query)
                || project
                    .requests
                    .iter()
                    .any(|request| request.name.to_lowercase().contains(&query))
        })
        .collect()
}

fn render_editor(
    view: &Entity<ApiSidebarView>,
    editor: &super::sidebar::ApiProjectEditor,
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

fn render_delete_confirm(
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

fn render_import_state(
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

fn render_project_row(
    view: &Entity<ApiSidebarView>,
    project: &transom_api::ApiProject,
    state: SidebarRowState,
    p: &Palette,
) -> AnyElement {
    let project_id = project.id;
    let title = project.display_name().to_string();
    let subtitle =
        rust_i18n::t!("api.project.request_count", count = project.request_count()).to_string();
    let owned_project = project.clone();
    let view_for_click = view.clone();
    let view_for_menu = view.clone();
    sidebar_row_band(
        ElementId::Name(format!("api-project-{project_id}").into()),
        state,
        p,
    )
    .on_click(move |_, _, cx| {
        view_for_click.update(cx, |this, cx| this.open_project(project_id, cx));
    })
    .context_menu(move |menu, _window, _cx| {
        let rename_view = view_for_menu.clone();
        let delete_view = view_for_menu.clone();
        let curl_view = view_for_menu.clone();
        let postman_view = view_for_menu.clone();
        let openapi_view = view_for_menu.clone();
        let transom_view = view_for_menu.clone();
        let project_for_menu = owned_project.clone();
        menu.item(menu_item(
            rust_i18n::t!("api.project.rename").to_string(),
            AppIcon::Ui(UiIcon::EDIT),
            move |_, window, cx| {
                rename_view.update(cx, |this, cx| this.start_rename(project_id, window, cx));
            },
        ))
        .item(menu_item(
            rust_i18n::t!("api.project.delete").to_string(),
            AppIcon::Ui(UiIcon::DELETE),
            move |_, _, cx| {
                delete_view.update(cx, |this, cx| this.request_delete(project_id, cx));
            },
        ))
        .item(menu_item(
            rust_i18n::t!("api.project.export_curl").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            {
                let project = project_for_menu.clone();
                move |_, _, cx| {
                    curl_view.update(cx, |_, cx| {
                        cx.emit(super::ApiSidebarEvent::ExportProject {
                            project: project.clone(),
                            format: ApiExportFormat::Curl,
                        });
                    });
                }
            },
        ))
        .item(menu_item(
            rust_i18n::t!("api.project.export_postman").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            {
                let project = project_for_menu.clone();
                move |_, _, cx| {
                    postman_view.update(cx, |_, cx| {
                        cx.emit(super::ApiSidebarEvent::ExportProject {
                            project: project.clone(),
                            format: ApiExportFormat::Postman,
                        });
                    });
                }
            },
        ))
        .item(menu_item(
            rust_i18n::t!("api.project.export_openapi").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            {
                let project = project_for_menu.clone();
                move |_, _, cx| {
                    openapi_view.update(cx, |_, cx| {
                        cx.emit(super::ApiSidebarEvent::ExportProject {
                            project: project.clone(),
                            format: ApiExportFormat::OpenApi,
                        });
                    });
                }
            },
        ))
        .item(menu_item(
            rust_i18n::t!("api.project.export_transom").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            move |_, _, cx| {
                transom_view.update(cx, |_, cx| {
                    cx.emit(super::ApiSidebarEvent::ExportProject {
                        project: project_for_menu.clone(),
                        format: ApiExportFormat::Transom,
                    });
                });
            },
        ))
    })
    .child(AppIconElement::new(AppIcon::Ui(UiIcon::WEB_SERVER)).small())
    .child(div().flex_1().min_w_0().truncate().child(title))
    .child(div().text_color(p.muted_foreground).child(subtitle))
    .into_any_element()
}
