//! API 工作区接口树视图。

use super::ApiWorkspace;
use super::tree_drag::{ApiDragPreview, ApiTreeDrag, ApiTreeDropTarget, reorder_overlay};
use super::view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space};
use crate::widgets::{
    IconButton, IconButtonSize, MenuItem, SidebarRowState, empty_state, menu_item,
    sidebar_group_row_band, sidebar_row_band,
};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_api::{ApiFolder, ApiRequest, ApiRequestId, method_for_request};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

pub(crate) fn render_tree(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view;
    let mut request_rows = v_flex().w_full();
    if let Some(editor) = &params.api_sidebar_state.editor {
        request_rows = request_rows.child(crate::api_client::sidebar::view::render_editor(
            params.api_sidebar,
            editor,
            p,
        ));
    }
    if let Some(project_id) = params.api_sidebar_state.pending_delete
        && let Some(project) = params.api_sidebar_state.registry.project(project_id)
    {
        request_rows = request_rows.child(crate::api_client::sidebar::view::render_delete_confirm(
            params.api_sidebar,
            project.display_name(),
            p,
        ));
    }
    if let Some(import) = &params.api_sidebar_state.pending_import {
        request_rows = request_rows.child(crate::api_client::sidebar::view::render_import_state(
            params.api_sidebar,
            import,
            p,
        ));
    }
    if let Some(error) = &params.api_sidebar_state.load_error {
        request_rows = request_rows.child(
            div()
                .mx(rems(space::XS))
                .p(rems(space::SM))
                .rounded(p.radius.md)
                .bg(p.surface.control_selected)
                .text_color(p.danger)
                .child(error.clone()),
        );
    }
    if let Some(editor) = params.ui.folder_editor.as_ref() {
        request_rows = request_rows.child(super::folder::render_folder_editor(view, editor, p));
    }
    if let Some(folder_id) = params.ui.pending_folder_delete
        && let Some(folder) = params
            .project
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
    {
        request_rows = request_rows.child(super::folder::render_folder_delete_confirm(
            view,
            folder,
            params
                .project
                .requests
                .iter()
                .filter(|request| request.folder_id == Some(folder_id))
                .count(),
            p,
        ));
    }
    for folder in &params.project.folders {
        let count = params
            .project
            .requests
            .iter()
            .filter(|request| request.folder_id == Some(folder.id))
            .count();
        let collapsed = params.ui.collapsed_folders.contains(&folder.id);
        request_rows = request_rows.child(render_folder_row(view, folder, collapsed, count, p));
        if collapsed {
            continue;
        }
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

    let body = if params.project.requests.is_empty() && params.project.folders.is_empty() {
        div()
            .p(rems(space::MD))
            .child(empty_state(
                AppIcon::Custom(CustomIcon::API),
                rust_i18n::t!("api.request.empty_title").to_string(),
                rust_i18n::t!("api.request.empty_hint").to_string(),
                p,
            ))
            .into_any_element()
    } else {
        div()
            .py(rems(space::XS))
            .child(request_rows)
            .into_any_element()
    };

    let view_for_new = view.clone();
    let view_for_new_folder = view.clone();
    let view_for_root_drop = view.clone();
    v_flex()
        .size_full()
        .min_w_0()
        .overflow_hidden()
        .bg(p.background)
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
                        .min_w_0()
                        .child(super::project_switcher::render_project_switcher(params, p)),
                )
                .child(
                    IconButton::new(
                        "api-folder-new",
                        AppIcon::Ui(UiIcon::NEW_FOLDER),
                        rust_i18n::t!("api.request.new_folder").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, window, cx| {
                        view_for_new_folder
                            .update(cx, |this, cx| this.start_create_folder(window, cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-request-new",
                        AppIcon::Ui(UiIcon::ADD),
                        rust_i18n::t!("api.request.new").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        view_for_new.update(cx, |this, cx| this.open_new_request_modal(cx));
                    }),
                ),
        )
        .child(
            div()
                .id("api-tree-scroll-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .on_drop(move |drag: &ApiTreeDrag, window, cx| {
                    let drag = *drag;
                    view_for_root_drop.update(cx, |this, cx| {
                        this.handle_tree_drop(drag, ApiTreeDropTarget::Root, cx)
                    });
                    window.refresh();
                })
                .child(body),
        )
        .into_any_element()
}

fn render_request_row(
    view: &Entity<ApiWorkspace>,
    request: &ApiRequest,
    selected_request: Option<ApiRequestId>,
    p: &Palette,
    nested: bool,
) -> AnyElement {
    const ROW_HEIGHT: Rems = rems(size::TREE_ROW_HEIGHT);
    const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);
    const INDENT_BASE: f32 = 6.0;
    const INDENT_STEP: f32 = 18.0;

    let view_for_click = view.clone();
    let view_for_menu = view.clone();
    let request_id = request.id;
    let current_folder = request.folder_id;
    let selected = selected_request == Some(request_id);
    let row_state = if selected {
        SidebarRowState::Current
    } else {
        SidebarRowState::Normal
    };

    let band = sidebar_row_band(
        ElementId::Name(format!("api-request-band-{request_id}").into()),
        row_state,
        p,
    )
    .child(AppIconElement::new(AppIcon::Custom(CustomIcon::API)).with_rems(ROW_ICON_SIZE))
    .child(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_sm()
            .truncate()
            .child(request.display_name().to_string()),
    )
    .child(
        div()
            .text_xs()
            .text_color(p.primary)
            .child(method_for_request(request)),
    )
    .child(reorder_overlay(
        view,
        ApiTreeDrag::Request(request_id),
        false,
        p,
    ));

    let drag_label = request.display_name().to_string();
    div()
        .id(ElementId::Name(format!("api-request-{request_id}").into()))
        .w_full()
        .h(ROW_HEIGHT)
        .pl(px(INDENT_BASE + if nested { INDENT_STEP } else { 0.0 }))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_drag(ApiTreeDrag::Request(request_id), move |_, _, _, cx| {
            cx.new(|_| ApiDragPreview::new(drag_label.clone(), CustomIcon::API.into()))
        })
        .on_click(move |_, window, cx| {
            view_for_click.update(cx, |this, cx| this.select_request(request_id, window, cx));
        })
        .context_menu(move |menu, _window, cx| {
            let menu_folders = view_for_menu.read(cx).project.folders.clone();
            let duplicate_view = view_for_menu.clone();
            let delete_view = view_for_menu.clone();
            let mut menu = menu.item(menu_item(
                rust_i18n::t!("api.request.duplicate").to_string(),
                AppIcon::Ui(UiIcon::COPY),
                move |_, window, cx| {
                    duplicate_view.update(cx, |this, cx| {
                        this.select_request(request_id, window, cx);
                        this.duplicate_selected(window, cx);
                    });
                },
            ));
            for folder in &menu_folders {
                let move_view = view_for_menu.clone();
                let folder_id = folder.id;
                let label = rust_i18n::t!("api.request.move_to_folder", name = folder.name.as_str())
                    .to_string();
                menu = menu.item(
                    MenuItem::new(label, AppIcon::Ui(UiIcon::FOLDER))
                        .disabled(current_folder == Some(folder_id))
                        .render(move |_, window, cx| {
                            move_view.update(cx, |this, cx| {
                                this.select_request(request_id, window, cx);
                                this.move_selected_to_folder(Some(folder_id), cx);
                            });
                        }),
                );
            }
            let move_root_view = view_for_menu.clone();
            menu = menu.item(
                MenuItem::new(
                    rust_i18n::t!("api.request.move_to_root").to_string(),
                    AppIcon::Ui(UiIcon::HOME_FOLDER),
                )
                .disabled(current_folder.is_none())
                .render(move |_, window, cx| {
                    move_root_view.update(cx, |this, cx| {
                        this.select_request(request_id, window, cx);
                        this.move_selected_to_folder(None, cx);
                    });
                }),
            );
            menu.item(menu_item(
                rust_i18n::t!("api.request.delete").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
                move |_, window, cx| {
                    delete_view.update(cx, |this, cx| {
                        this.select_request(request_id, window, cx);
                        this.delete_selected(window, cx);
                    });
                },
            ))
        })
        .child(band)
        .into_any_element()
}

fn render_folder_row(
    view: &Entity<ApiWorkspace>,
    folder: &ApiFolder,
    collapsed: bool,
    count: usize,
    p: &Palette,
) -> AnyElement {
    const ROW_HEIGHT: Rems = rems(size::TREE_ROW_HEIGHT);
    const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);
    const ROW_CHEVRON_SIZE: Rems = rems(size::TREE_CHEVRON);
    const INDENT_BASE: f32 = 6.0;

    let toggle_view = view.clone();
    let rename_view = view.clone();
    let delete_view = view.clone();
    let folder_id = folder.id;
    let into_bg = p.drop_target;
    let anchor = ApiTreeDrag::Folder(folder_id);
    let drop_view = view.clone();
    let band = sidebar_group_row_band(
        ElementId::Name(format!("api-folder-band-{folder_id}").into()),
        p,
    )
    .drag_over::<ApiTreeDrag>(move |style, _, _, _| style.bg(into_bg))
    .on_drop(move |drag: &ApiTreeDrag, window, cx| {
        let drag = *drag;
        drop_view.update(cx, |this, cx| {
            this.handle_tree_drop(drag, ApiTreeDropTarget::Folder(folder_id), cx)
        });
        window.refresh();
    })
    .child(
        AppIconElement::new(if collapsed {
            UiIcon::CHEVRON_RIGHT
        } else {
            UiIcon::CHEVRON_DOWN
        })
        .with_rems(ROW_CHEVRON_SIZE),
    )
    .child(AppIconElement::new(UiIcon::FOLDER).with_rems(ROW_ICON_SIZE))
    .child(
        div()
            .flex_1()
            .overflow_hidden()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .child(folder.name.clone()),
    )
    .child(
        div()
            .text_xs()
            .text_color(p.text.muted)
            .child(count.to_string()),
    )
    .child(reorder_overlay(view, anchor, true, p));

    let drag_label = folder.name.clone();
    div()
        .id(ElementId::Name(format!("api-folder-{folder_id}").into()))
        .w_full()
        .h(ROW_HEIGHT)
        .pl(px(INDENT_BASE))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_drag(anchor, move |_, _, _, cx| {
            cx.new(|_| ApiDragPreview::new(drag_label.clone(), UiIcon::FOLDER.into()))
        })
        .on_click(move |_, _, cx| {
            toggle_view.update(cx, |this, cx| this.toggle_folder_collapsed(folder_id, cx));
        })
        .context_menu(move |menu, _, _| {
            let rename_view = rename_view.clone();
            let delete_view = delete_view.clone();
            menu.item(menu_item(
                rust_i18n::t!("api.request.rename_folder").to_string(),
                AppIcon::Ui(UiIcon::EDIT),
                move |_, window, cx| {
                    rename_view.update(cx, |this, cx| {
                        this.start_rename_folder(folder_id, window, cx)
                    });
                },
            ))
            .item(menu_item(
                rust_i18n::t!("api.request.delete_folder").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
                move |_, _, cx| {
                    delete_view.update(cx, |this, cx| this.request_delete_folder(folder_id, cx));
                },
            ))
        })
        .child(band)
        .into_any_element()
}
