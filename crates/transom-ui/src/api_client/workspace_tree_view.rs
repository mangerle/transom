//! API 工作区接口树视图。

use super::ApiWorkspace;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use crate::widgets::{IconButton, IconButtonSize, MenuItem, action_button, list_row, menu_item};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiFolder, ApiRequest, ApiRequestId, method_for_request};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(crate) fn render_tree(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view;
    let mut request_rows = v_flex().w_full().gap(rems(space::XXS));
    if let Some(editor) = params.folder_editor {
        request_rows = request_rows.child(render_folder_editor(view, editor, p));
    }
    if let Some(folder_id) = params.pending_folder_delete
        && let Some(folder) = params
            .project
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
    {
        request_rows = request_rows.child(render_folder_delete_confirm(
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
        request_rows = request_rows.child(render_folder_row(view, folder, p));
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
    let view_for_new_folder = view.clone();
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
    let current_folder = request.folder_id;
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

fn render_folder_row(view: &Entity<ApiWorkspace>, folder: &ApiFolder, p: &Palette) -> AnyElement {
    let rename_view = view.clone();
    let delete_view = view.clone();
    let folder_id = folder.id;
    h_flex()
        .id(ElementId::Name(format!("api-folder-{folder_id}").into()))
        .h(rems(size::TREE_BAND_HEIGHT))
        .px(rems(space::SM))
        .items_center()
        .gap(rems(space::XS))
        .bg(p.list_head)
        .text_color(p.muted_foreground)
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
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).xsmall())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(folder.name.clone()),
        )
        .into_any_element()
}

fn render_folder_editor(
    view: &Entity<ApiWorkspace>,
    editor: &super::workspace_folder::FolderEditor,
    p: &Palette,
) -> AnyElement {
    let submit_view = view.clone();
    let cancel_view = view.clone();
    let row = h_flex()
        .mx(rems(space::XS))
        .my(rems(space::XXS))
        .gap(rems(space::XS))
        .items_center()
        .child(Input::new(&editor.input).flex_1().min_w_0())
        .child(
            IconButton::new(
                "api-folder-editor-submit",
                AppIcon::Ui(UiIcon::CHECK),
                rust_i18n::t!("api.request.save_folder").to_string(),
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                submit_view.update(cx, |this, cx| this.submit_folder_editor(cx));
            }),
        )
        .child(
            IconButton::new(
                "api-folder-editor-cancel",
                AppIcon::Ui(UiIcon::CLOSE),
                rust_i18n::t!("api.project.cancel").to_string(),
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                cancel_view.update(cx, |this, cx| this.cancel_folder_editor(cx));
            }),
        );
    let mut content = v_flex().child(row);
    if let Some(error) = &editor.error {
        content = content.child(
            div()
                .px(rems(space::SM))
                .text_color(p.danger)
                .child(error.clone()),
        );
    }
    content.into_any_element()
}

fn render_folder_delete_confirm(
    view: &Entity<ApiWorkspace>,
    folder: &ApiFolder,
    request_count: usize,
    p: &Palette,
) -> AnyElement {
    let confirm_view = view.clone();
    let cancel_view = view.clone();
    v_flex()
        .mx(rems(space::XS))
        .my(rems(space::XXS))
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.danger)
        .child(
            div().text_color(p.foreground).child(
                rust_i18n::t!(
                    "api.request.delete_folder_confirm",
                    name = folder.name.as_str()
                )
                .to_string(),
            ),
        )
        .child(div().text_color(p.muted_foreground).child(
            rust_i18n::t!("api.request.delete_folder_hint", count = request_count).to_string(),
        ))
        .child(
            h_flex()
                .gap(rems(space::XS))
                .child(
                    action_button(
                        "api-folder-delete-confirm",
                        AppIcon::Ui(UiIcon::DELETE),
                        rust_i18n::t!("api.project.delete").to_string(),
                    )
                    .danger()
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |this, cx| this.confirm_delete_folder(cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-folder-delete-cancel",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("api.project.cancel").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |this, cx| this.cancel_delete_folder(cx));
                    }),
                ),
        )
        .into_any_element()
}
