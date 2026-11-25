//! 请求表单体表格的渲染。

use super::form::FormEditor;
use super::form_file::{FormFileIssue, FormFileStatus, file_display_name};
use super::table::{
    ColumnWidth, table_action_slot, table_custom_cell, table_header_row, table_input_cell,
    table_leading_toggle_slot, table_remove_button, table_row_container, table_scroll_frame,
};
use crate::format::format_compact_size;
use crate::theme::tokens::{control, space, typography};
use crate::widgets::{IconButton, IconButtonSize, MenuItem, action_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::ApiFormFieldKind;
use transom_ui_icons::{AppIcon, AppIconElement, FileTypeIcon, UiIcon};

impl Render for FormEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let allow_files = self.allow_files;
        let last = self.rows.len().saturating_sub(1);
        let mut columns = vec![(
            rust_i18n::t!("api.request.form_name").to_string(),
            ColumnWidth::Relative(0.24),
        )];
        if allow_files {
            columns.push((
                rust_i18n::t!("api.request.form_type").to_string(),
                ColumnWidth::Fixed(rems(7.0)),
            ));
        }
        columns.push((
            rust_i18n::t!("api.request.form_value").to_string(),
            ColumnWidth::Flex,
        ));
        columns.push((
            rust_i18n::t!("api.request.parameter_description").to_string(),
            ColumnWidth::Relative(0.24),
        ));
        let header = table_header_row(
            border,
            theme.muted,
            theme.muted_foreground,
            true,
            true,
            columns,
        );

        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let add = cx.entity().downgrade();
            let toggle = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            let kind_entity = cx.entity().downgrade();
            let file_entity = cx.entity().downgrade();
            let clear_file = cx.entity().downgrade();
            let kind = if allow_files {
                row.kind
            } else {
                ApiFormFieldKind::Text
            };
            let value_child = if kind == ApiFormFieldKind::File {
                render_file_value(FormFileValueParams {
                    id,
                    path: row.file_path.as_deref(),
                    status: &row.file_status,
                    file_entity,
                    clear_entity: clear_file,
                    danger: theme.danger,
                    muted: theme.muted_foreground,
                })
            } else {
                Input::new(&row.value)
                    .small()
                    .appearance(false)
                    .into_any_element()
            };
            table_row_container("api-form-row", id, !blank && !row.enabled, border)
                .child(table_leading_toggle_slot(
                    "api-form-add",
                    "api-form-enabled",
                    id,
                    blank,
                    row.enabled,
                    rust_i18n::t!("api.request.add_field").to_string(),
                    move |_, window, cx| {
                        let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                    },
                    move |_, _, cx| {
                        let _ = toggle.update(cx, |this, cx| this.toggle(id, cx));
                    },
                ))
                .child(table_input_cell(
                    &row.name,
                    ColumnWidth::Relative(0.24),
                    border,
                ))
                .when(allow_files, |row_bar| {
                    let kind = row.kind;
                    let entity = kind_entity.clone();
                    row_bar.child(table_custom_cell(
                        ColumnWidth::Fixed(rems(7.0)),
                        border,
                        action_button(("api-form-kind", id), kind_icon(kind), kind_label(kind))
                            .ghost()
                            .dropdown_caret(true)
                            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                menu.item(
                                    MenuItem::new(
                                        rust_i18n::t!("api.request.value_text").to_string(),
                                        AppIcon::FileType(FileTypeIcon::TEXT),
                                    )
                                    .checked(kind == ApiFormFieldKind::Text)
                                    .render({
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            let _ = entity.update(cx, |this, cx| {
                                                this.set_kind(
                                                    id,
                                                    ApiFormFieldKind::Text,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }
                                    }),
                                )
                                .item(
                                    MenuItem::new(
                                        rust_i18n::t!("api.request.value_file").to_string(),
                                        AppIcon::Ui(UiIcon::ADD_FILE),
                                    )
                                    .checked(kind == ApiFormFieldKind::File)
                                    .render({
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            let _ = entity.update(cx, |this, cx| {
                                                this.set_kind(
                                                    id,
                                                    ApiFormFieldKind::File,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }
                                    }),
                                )
                            }),
                    ))
                })
                .child(table_custom_cell(ColumnWidth::Flex, border, value_child))
                .child(table_input_cell(
                    &row.description,
                    ColumnWidth::Relative(0.24),
                    border,
                ))
                .child(table_action_slot((!blank).then(|| {
                    table_remove_button(
                        "api-form-remove",
                        id,
                        rust_i18n::t!("api.request.remove_field").to_string(),
                        move |_, window, cx| {
                            let _ = remove.update(cx, |this, cx| this.remove(id, window, cx));
                        },
                    )
                })))
                .into_any_element()
        });

        table_scroll_frame("api-form-editor", border, header, rows)
    }
}

fn kind_label(kind: ApiFormFieldKind) -> String {
    match kind {
        ApiFormFieldKind::Text => rust_i18n::t!("api.request.value_text").to_string(),
        ApiFormFieldKind::File => rust_i18n::t!("api.request.value_file").to_string(),
    }
}

fn kind_icon(kind: ApiFormFieldKind) -> AppIcon {
    match kind {
        ApiFormFieldKind::Text => AppIcon::FileType(FileTypeIcon::TEXT),
        ApiFormFieldKind::File => AppIcon::Ui(UiIcon::ADD_FILE),
    }
}

struct FormFileValueParams<'a> {
    id: u64,
    path: Option<&'a str>,
    status: &'a FormFileStatus,
    file_entity: WeakEntity<FormEditor>,
    clear_entity: WeakEntity<FormEditor>,
    danger: Hsla,
    muted: Hsla,
}

fn render_file_value(params: FormFileValueParams<'_>) -> AnyElement {
    let FormFileValueParams {
        id,
        path,
        status,
        file_entity,
        clear_entity,
        danger,
        muted,
    } = params;
    let Some(path) = path else {
        let entity = file_entity;
        return h_flex()
            .w_full()
            .min_w_0()
            .child(
                action_button(
                    ("api-form-file", id),
                    AppIcon::Ui(UiIcon::ADD_FILE),
                    rust_i18n::t!("api.request.choose_file").to_string(),
                )
                .small()
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| this.pick_file(id, window, cx));
                }),
            )
            .into_any_element();
    };

    let replace_entity = file_entity.clone();
    let clear_entity_for_click = clear_entity;
    let (status_text, invalid) = file_status_text(status);
    let tooltip_path = path.to_string();
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(rems(space::XS))
        .child(
            AppIconElement::new(AppIcon::for_file(path))
                .with_rems(rems(control::COMPACT_ICON))
                .flex_none(),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .id(("api-form-file-name", id))
                        .min_w_0()
                        .truncate()
                        .child(file_display_name(path))
                        .tooltip(move |window, cx| {
                            Tooltip::new(tooltip_path.clone()).build(window, cx)
                        }),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rems(typography::CAPTION))
                        .text_color(if invalid { danger } else { muted })
                        .child(status_text),
                ),
        )
        .child(
            IconButton::new(
                ("api-form-file-replace", id),
                AppIcon::Ui(UiIcon::ADD_FILE),
                rust_i18n::t!("api.request.replace_file").to_string(),
            )
            .size(IconButtonSize::RowAction)
            .render()
            .on_click(move |_, window, cx| {
                let _ = replace_entity.update(cx, |this, cx| this.pick_file(id, window, cx));
            }),
        )
        .child(
            IconButton::new(
                ("api-form-file-clear", id),
                AppIcon::Ui(UiIcon::DELETE),
                rust_i18n::t!("api.request.clear_file").to_string(),
            )
            .size(IconButtonSize::RowAction)
            .render()
            .on_click(move |_, _, cx| {
                let _ = clear_entity_for_click.update(cx, |this, cx| this.clear_file(id, cx));
            }),
        )
        .into_any_element()
}

fn file_status_text(status: &FormFileStatus) -> (String, bool) {
    match status {
        FormFileStatus::Unknown => (String::new(), false),
        FormFileStatus::Checking => (
            rust_i18n::t!("api.request.file_checking").to_string(),
            false,
        ),
        FormFileStatus::Ready { size } => (format_compact_size(*size), false),
        FormFileStatus::Invalid(issue) => match issue {
            FormFileIssue::Missing => (rust_i18n::t!("api.request.file_missing").to_string(), true),
            FormFileIssue::NotFile => (
                rust_i18n::t!("api.request.file_not_regular").to_string(),
                true,
            ),
            FormFileIssue::TooLarge { size, limit } => (
                rust_i18n::t!(
                    "api.request.file_too_large",
                    size = format_compact_size(*size),
                    limit = format_compact_size(*limit)
                )
                .to_string(),
                true,
            ),
            FormFileIssue::Unreadable { reason } => (
                rust_i18n::t!("api.request.file_unreadable", reason = reason).to_string(),
                true,
            ),
        },
    }
}
