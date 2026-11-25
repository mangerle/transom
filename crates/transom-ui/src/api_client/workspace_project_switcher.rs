//! API 请求树顶部的项目切换与管理菜单。

use super::workspace_view::ApiWorkspaceParams;
use super::{ApiSidebarEvent, ApiSidebarView};
use crate::palette::Palette;
use crate::widgets::{MenuItem, action_button};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::*;
use transom_api::ApiExportFormat;
use transom_ui_icons::{AppIcon, UiIcon};

pub(super) fn render_project_switcher(params: &ApiWorkspaceParams<'_>, _p: &Palette) -> AnyElement {
    let view = params.api_sidebar.clone();
    let active_project = params.project.id;
    let label = params.project.display_name().to_string();
    action_button("api-project-switcher", AppIcon::Ui(UiIcon::PROJECT), label)
        .ghost()
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, cx| {
            let projects = view
                .read(cx)
                .state
                .registry
                .projects
                .iter()
                .map(|project| (project.id, project.display_name().to_string()))
                .collect::<Vec<_>>();
            let mut menu = menu;
            for (project_id, name) in projects {
                let switch_view = view.clone();
                menu = menu.item(
                    MenuItem::new(name, AppIcon::Ui(UiIcon::WEB_SERVER))
                        .checked(project_id == active_project)
                        .render(move |_, _, cx| {
                            switch_view.update(cx, |view, cx| view.open_project(project_id, cx));
                        }),
                );
            }
            let create_view = view.clone();
            let import_view = view.clone();
            let rename_view = view.clone();
            let delete_view = view.clone();
            menu.separator()
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("api.sidebar.new_project").to_string(),
                    AppIcon::Ui(UiIcon::ADD),
                    move |_, window, cx| {
                        create_view.update(cx, |view, cx| view.start_create(window, cx));
                    },
                ))
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("api.sidebar.import_project").to_string(),
                    AppIcon::Ui(UiIcon::DOWNLOAD),
                    move |_, _, cx| {
                        import_view.update(cx, |_, cx| {
                            cx.emit(ApiSidebarEvent::ImportProject);
                            cx.notify();
                        });
                    },
                ))
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("api.project.rename").to_string(),
                    AppIcon::Ui(UiIcon::EDIT),
                    move |_, window, cx| {
                        rename_view
                            .update(cx, |view, cx| view.start_rename(active_project, window, cx));
                    },
                ))
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("api.project.delete").to_string(),
                    AppIcon::Ui(UiIcon::DELETE),
                    move |_, _, cx| {
                        delete_view.update(cx, |view, cx| view.request_delete(active_project, cx));
                    },
                ))
                .separator()
                .item(export_item(
                    &view,
                    active_project,
                    ApiExportFormat::Curl,
                    rust_i18n::t!("api.project.export_curl").to_string(),
                ))
                .item(export_item(
                    &view,
                    active_project,
                    ApiExportFormat::Postman,
                    rust_i18n::t!("api.project.export_postman").to_string(),
                ))
                .item(export_item(
                    &view,
                    active_project,
                    ApiExportFormat::OpenApi,
                    rust_i18n::t!("api.project.export_openapi").to_string(),
                ))
                .item(export_item(
                    &view,
                    active_project,
                    ApiExportFormat::Transom,
                    rust_i18n::t!("api.project.export_transom").to_string(),
                ))
        })
        .into_any_element()
}

fn export_item(
    view: &Entity<ApiSidebarView>,
    project_id: transom_api::ApiProjectId,
    format: ApiExportFormat,
    label: String,
) -> gpui_kit::component::menu::PopupMenuItem {
    let view = view.clone();
    crate::widgets::menu_item(label, AppIcon::Ui(UiIcon::DOWNLOAD), move |_, _, cx| {
        view.update(cx, |view, cx| view.export_project(project_id, format, cx));
    })
}
