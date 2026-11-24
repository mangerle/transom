//! 请求表单体表格的渲染。

use super::form_editor::FormEditor;
use crate::theme::tokens::{size, space, typography};
use crate::widgets::{IconButton, action_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::Path as FsPath;
use transom_api::ApiFormFieldKind;
use transom_ui_icons::{AppIcon, UiIcon};

impl Render for FormEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let allow_files = self.allow_files;
        let last = self.rows.len().saturating_sub(1);
        let header = h_flex()
            .h(rems(size::LARGE_HEADER_HEIGHT))
            .bg(theme.muted)
            .text_size(rems(typography::CAPTION))
            .text_color(theme.muted_foreground)
            .border_color(border)
            .border_b_1()
            .child(div().w(rems(3.0)).flex_none())
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(header_cell(
                        rust_i18n::t!("api.request.form_name").to_string(),
                        relative(0.24),
                        border,
                    ))
                    .when(allow_files, |row| {
                        row.child(header_cell(
                            rust_i18n::t!("api.request.form_type").to_string(),
                            rems(7.0),
                            border,
                        ))
                    })
                    .child(flex_header_cell(
                        rust_i18n::t!("api.request.form_value").to_string(),
                        border,
                    ))
                    .child(header_cell(
                        rust_i18n::t!("api.request.parameter_description").to_string(),
                        relative(0.24),
                        border,
                    )),
            )
            .child(div().w(rems(3.0)).flex_none());

        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let toggle = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            let kind_entity = cx.entity().downgrade();
            let file_entity = cx.entity().downgrade();
            let file_name = file_name(&row.value.read(cx).value());
            h_flex()
                .id(("api-form-row", id))
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .items_center()
                .border_color(border)
                .border_b_1()
                .when(!blank && !row.enabled, |row| row.opacity(0.45))
                .child(
                    div()
                        .w(rems(3.0))
                        .flex_none()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(blank, |cell| {
                            let add = cx.entity().downgrade();
                            cell.child(
                                IconButton::new(
                                    ("api-form-add", id),
                                    AppIcon::Ui(UiIcon::ADD),
                                    rust_i18n::t!("api.request.add_field").to_string(),
                                )
                                .render()
                                .on_click(move |_, window, cx| {
                                    let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                                }),
                            )
                        })
                        .when(!blank, |cell| {
                            cell.child(
                                Checkbox::new(("api-form-enabled", id))
                                    .checked(row.enabled)
                                    .on_click(move |_, _, cx| {
                                        let _ = toggle.update(cx, |this, cx| this.toggle(id, cx));
                                    }),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            div()
                                .w(relative(0.24))
                                .flex_none()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .px(rems(space::SM))
                                .border_l_1()
                                .border_color(border)
                                .child(Input::new(&row.name).small().appearance(false)),
                        )
                        .when(allow_files, |row_bar| {
                            let kind = row.kind;
                            let entity = kind_entity.clone();
                            row_bar.child(
                                div()
                                    .w(rems(7.0))
                                    .flex_none()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .px(rems(space::XS))
                                    .border_l_1()
                                    .border_color(border)
                                    .child(
                                        Button::new(("api-form-kind", id))
                                            .ghost()
                                            .small()
                                            .label(kind_label(kind))
                                            .dropdown_caret(true)
                                            .dropdown_menu_with_anchor(
                                                Anchor::TopRight,
                                                move |menu, _, _| {
                                                    menu.item(
                                                        PopupMenuItem::new(
                                                            rust_i18n::t!("api.request.value_text")
                                                                .to_string(),
                                                        )
                                                        .checked(kind == ApiFormFieldKind::Text)
                                                        .on_click({
                                                            let entity = entity.clone();
                                                            move |_, window, cx| {
                                                                let _ = entity.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        this.set_kind(
                                                                            id,
                                                                            ApiFormFieldKind::Text,
                                                                            window,
                                                                            cx,
                                                                        )
                                                                    },
                                                                );
                                                            }
                                                        }),
                                                    )
                                                    .item(
                                                        PopupMenuItem::new(
                                                            rust_i18n::t!("api.request.value_file")
                                                                .to_string(),
                                                        )
                                                        .checked(kind == ApiFormFieldKind::File)
                                                        .on_click({
                                                            let entity = entity.clone();
                                                            move |_, window, cx| {
                                                                let _ = entity.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        this.set_kind(
                                                                            id,
                                                                            ApiFormFieldKind::File,
                                                                            window,
                                                                            cx,
                                                                        )
                                                                    },
                                                                );
                                                            }
                                                        }),
                                                    )
                                                },
                                            ),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .px(rems(space::SM))
                                .border_l_1()
                                .border_color(border)
                                .when(row.kind == ApiFormFieldKind::File, |cell| {
                                    let entity = file_entity.clone();
                                    cell.child(
                                        action_button(
                                            ("api-form-file", id),
                                            AppIcon::Ui(UiIcon::ADD_FILE),
                                            file_name.unwrap_or_else(|| {
                                                rust_i18n::t!("api.request.choose_file").to_string()
                                            }),
                                        )
                                        .small()
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = entity.update(cx, |this, cx| {
                                                    this.pick_file(id, window, cx)
                                                });
                                            },
                                        ),
                                    )
                                })
                                .when(row.kind == ApiFormFieldKind::Text, |cell| {
                                    cell.child(Input::new(&row.value).small().appearance(false))
                                }),
                        )
                        .child(
                            div()
                                .w(relative(0.24))
                                .flex_none()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .px(rems(space::SM))
                                .border_l_1()
                                .border_color(border)
                                .child(Input::new(&row.description).small().appearance(false)),
                        ),
                )
                .child(
                    div()
                        .w(rems(3.0))
                        .flex_none()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!blank, |cell| {
                            cell.child(
                                IconButton::new(
                                    ("api-form-remove", id),
                                    AppIcon::Ui(UiIcon::DELETE),
                                    rust_i18n::t!("api.request.remove_field").to_string(),
                                )
                                .render()
                                .on_click(move |_, window, cx| {
                                    let _ =
                                        remove.update(cx, |this, cx| this.remove(id, window, cx));
                                }),
                            )
                        }),
                )
                .into_any_element()
        });

        v_flex()
            .id("api-form-editor")
            .flex_1()
            .min_h_0()
            .w_full()
            .p(rems(space::SM))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .child(header)
                    .children(rows),
            )
            .into_any_element()
    }
}

fn flex_header_cell(label: String, border: Hsla) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .px(rems(space::SM))
        .flex()
        .items_center()
        .border_l_1()
        .border_color(border)
        .child(label)
        .into_any_element()
}

fn kind_label(kind: ApiFormFieldKind) -> String {
    match kind {
        ApiFormFieldKind::Text => rust_i18n::t!("api.request.value_text").to_string(),
        ApiFormFieldKind::File => rust_i18n::t!("api.request.value_file").to_string(),
    }
}

fn file_name(value: &str) -> Option<String> {
    let path = value.trim().strip_prefix("file://").unwrap_or(value.trim());
    if path.is_empty() {
        return None;
    }
    FsPath::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn header_cell(
    label: String,
    width: impl Into<gpui_kit::Length> + Clone,
    border: Hsla,
) -> AnyElement {
    div()
        .w(width)
        .flex_none()
        .min_w_0()
        .h_full()
        .px(rems(space::SM))
        .flex()
        .items_center()
        .border_l_1()
        .border_color(border)
        .child(label)
        .into_any_element()
}
