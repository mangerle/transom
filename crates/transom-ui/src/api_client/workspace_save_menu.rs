//! 请求保存按钮与保存位置弹层。

use super::ApiWorkspace;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::*;
use transom_api::{ApiFolder, ApiFolderId};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(super) fn render_save_controls(params: &ApiWorkspaceParams<'_>, _p: &Palette) -> AnyElement {
    let save_view = params.view.clone();
    let open_view = params.view.clone();
    let menu_view = params.view.clone();
    let search = params.save_destination_search.clone();
    let menu_search = search.clone();
    let folders = params.project.folders.clone();
    let open = params.save_destination_open;
    let trigger = IconButton::new(
        "api-request-save-destination",
        AppIcon::Ui(UiIcon::CHEVRON_DOWN),
        rust_i18n::t!("api.request.save_destination").to_string(),
    )
    .size(IconButtonSize::RowAction)
    .selected(open)
    .render();
    let popover = Popover::new("api-request-save-destination-popover")
        .anchor(Anchor::TopRight)
        .offset(px(4.0))
        .open(open)
        .on_open_change(move |next, window, cx| {
            let next = *next;
            open_view.update(cx, |this, cx| {
                this.save_destination_open = next;
                if next {
                    let focus = this
                        .editors
                        .save_destination_search
                        .read(cx)
                        .focus_handle(cx);
                    focus.focus(window, cx);
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_, _, cx| {
            render_save_destination_menu(&menu_view, &menu_search, &folders, cx)
        });

    h_flex()
        .gap(rems(space::XXS))
        .items_center()
        .child(
            Button::new("api-request-save")
                .secondary()
                .small()
                .label(rust_i18n::t!("api.request.save").to_string())
                .on_click(move |_, _, cx| {
                    save_view.update(cx, |this, cx| this.save_project(cx));
                }),
        )
        .child(popover)
        .into_any_element()
}

fn render_save_destination_menu(
    view: &Entity<ApiWorkspace>,
    search: &Entity<gpui_kit::component::input::InputState>,
    folders: &[ApiFolder],
    cx: &App,
) -> AnyElement {
    let query = search.read(cx).value().trim().to_lowercase();
    let root_matches = query.is_empty()
        || rust_i18n::t!("api.request.save_root")
            .to_lowercase()
            .contains(&query);
    let mut rows = Vec::with_capacity(folders.len().saturating_add(1));
    if root_matches {
        rows.push(destination_row(
            "api-save-root",
            rust_i18n::t!("api.request.save_root").to_string(),
            AppIcon::Ui(UiIcon::HOME_FOLDER),
            view,
            None,
        ));
    }
    for folder in folders {
        if !query.is_empty() && !folder.name.to_lowercase().contains(&query) {
            continue;
        }
        rows.push(destination_row(
            format!("api-save-folder-{}", folder.id),
            folder.name.clone(),
            AppIcon::Ui(UiIcon::FOLDER),
            view,
            Some(folder.id),
        ));
    }
    v_flex()
        .w(rems(16.0))
        .p(rems(space::XS))
        .gap(rems(space::XXS))
        .child(Input::new(search).small().appearance(false))
        .children(rows)
        .into_any_element()
}

fn destination_row(
    id: impl Into<ElementId>,
    label: String,
    icon: AppIcon,
    view: &Entity<ApiWorkspace>,
    folder_id: Option<ApiFolderId>,
) -> AnyElement {
    let view = view.clone();
    Button::new(id)
        .ghost()
        .small()
        .w_full()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap(rems(space::XS))
                .child(AppIconElement::new(icon).small())
                .child(div().flex_1().min_w_0().truncate().child(label)),
        )
        .on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
                this.save_selected_to_destination(folder_id, window, cx)
            });
        })
        .into_any_element()
}

impl ApiWorkspace {
    fn save_selected_to_destination(
        &mut self,
        folder_id: Option<ApiFolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selected_to_folder(folder_id, cx);
        self.save_project(cx);
        self.save_destination_open = false;
        self.editors
            .save_destination_search
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        cx.notify();
    }
}
