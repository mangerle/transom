//! 项目环境变量逐行编辑器。

use crate::theme::tokens::{size, space, typography};
use crate::widgets::IconButton;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::ApiVariable;
use transom_ui_icons::{AppIcon, UiIcon};

pub(crate) enum EnvironmentEditorEvent {
    Changed(Vec<ApiVariable>),
}

struct VariableRow {
    id: u64,
    enabled: bool,
    secret: bool,
    name: Entity<InputState>,
    value: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

pub(crate) struct EnvironmentEditor {
    rows: Vec<VariableRow>,
    next_id: u64,
    syncing: bool,
}

fn blank_variable() -> ApiVariable {
    ApiVariable {
        name: String::new(),
        value: String::new(),
        enabled: true,
        secret: false,
    }
}

impl EventEmitter<EnvironmentEditorEvent> for EnvironmentEditor {}

impl EnvironmentEditor {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            rows: Vec::with_capacity(8),
            next_id: 0,
            syncing: false,
        };
        editor.push_row(blank_variable(), window, cx);
        editor
    }

    pub(crate) fn set_variables(
        &mut self,
        variables: &[ApiVariable],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncing = true;
        self.rows.clear();
        for variable in variables {
            self.push_row(variable.clone(), window, cx);
        }
        self.push_row(blank_variable(), window, cx);
        self.syncing = false;
        cx.notify();
    }

    pub(crate) fn variables(&self, cx: &App) -> Vec<ApiVariable> {
        self.rows
            .iter()
            .map(|row| ApiVariable {
                name: row.name.read(cx).value().trim().to_string(),
                value: row.value.read(cx).value().to_string(),
                enabled: row.enabled,
                secret: row.secret,
            })
            .filter(|variable| !variable.name.is_empty() || !variable.value.is_empty())
            .collect()
    }

    fn push_row(&mut self, variable: ApiVariable, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.environment_name_placeholder").to_string())
                .default_value(variable.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.environment_value_placeholder").to_string())
                .default_value(variable.value)
        });
        let watch_name =
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            });
        let watch_value =
            cx.subscribe_in(&value, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            });
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(VariableRow {
            id: self.next_id,
            enabled: variable.enabled,
            secret: variable.secret,
            name,
            value,
            _subscriptions: [watch_name, watch_value],
        });
    }

    fn on_input(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(event, InputEvent::Change) || self.syncing {
            return;
        }
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }

    fn ensure_trailing_blank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.rows.last() else {
            self.push_row(blank_variable(), window, cx);
            return;
        };
        if !last.name.read(cx).value().is_empty() || !last.value.read(cx).value().is_empty() {
            self.push_row(blank_variable(), window, cx);
        }
    }

    fn report(&mut self, cx: &mut Context<Self>) {
        cx.emit(EnvironmentEditorEvent::Changed(self.variables(cx)));
        cx.notify();
    }

    fn toggle_enabled(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.enabled = !row.enabled;
            self.report(cx);
        }
    }

    fn toggle_secret(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.secret = !row.secret;
            self.report(cx);
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(blank_variable(), window, cx);
        self.report(cx);
    }

    fn remove(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }
}

impl Render for EnvironmentEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let last = self.rows.len().saturating_sub(1);
        let header = h_flex()
            .h(rems(size::LARGE_HEADER_HEIGHT))
            .bg(theme.muted)
            .text_size(rems(typography::CAPTION))
            .text_color(theme.muted_foreground)
            .border_color(border)
            .border_b_1()
            .child(div().w(rems(3.0)).flex_none())
            .child(header_cell(
                rust_i18n::t!("api.request.environment_name").to_string(),
                relative(0.32),
                border,
            ))
            .child(flex_header_cell(
                rust_i18n::t!("api.request.environment_value").to_string(),
                border,
            ))
            .child(
                div().w(rems(6.5)).flex_none().child(
                    div()
                        .h_full()
                        .px(rems(space::SM))
                        .flex()
                        .items_center()
                        .border_l_1()
                        .border_color(border)
                        .child(rust_i18n::t!("api.request.environment_secret").to_string()),
                ),
            )
            .child(div().w(rems(3.0)).flex_none());
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let enabled = cx.entity().downgrade();
            let secret = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            h_flex()
                .id(("api-env-row", id))
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
                                    ("api-env-add", id),
                                    AppIcon::Ui(UiIcon::ADD),
                                    rust_i18n::t!("api.request.add_environment").to_string(),
                                )
                                .render()
                                .on_click(move |_, window, cx| {
                                    let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                                }),
                            )
                        })
                        .when(!blank, |cell| {
                            cell.child(
                                Checkbox::new(("api-env-enabled", id))
                                    .checked(row.enabled)
                                    .on_click(move |_, _, cx| {
                                        let _ = enabled
                                            .update(cx, |this, cx| this.toggle_enabled(id, cx));
                                    }),
                            )
                        }),
                )
                .child(
                    div()
                        .w(relative(0.32))
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
                        .child(Input::new(&row.value).small().appearance(false)),
                )
                .child(
                    div()
                        .w(rems(6.5))
                        .flex_none()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_l_1()
                        .border_color(border)
                        .child(
                            Checkbox::new(("api-env-secret", id))
                                .checked(row.secret)
                                .on_click(move |_, _, cx| {
                                    let _ =
                                        secret.update(cx, |this, cx| this.toggle_secret(id, cx));
                                }),
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
                                    ("api-env-remove", id),
                                    AppIcon::Ui(UiIcon::DELETE),
                                    rust_i18n::t!("api.request.remove_environment").to_string(),
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
            .id("api-environment-editor")
            .size_full()
            .min_h_0()
            .p(rems(space::SM))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .rounded(rems(space::XS))
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .child(header)
                    .children(rows),
            )
            .into_any_element()
    }
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
