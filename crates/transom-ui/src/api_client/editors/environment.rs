//! 项目环境变量逐行编辑器。

use super::table::{
    ColumnWidth, LeadingToggleSlotParams, table_action_slot, table_checkbox_cell, table_header_row,
    table_input_cell, table_leading_toggle_slot, table_remove_button, table_row_container,
    table_scroll_frame,
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use transom_api::ApiVariable;

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
        let blank = blank_variable();
        let target_len = variables.len() + 1;
        let reuse_len = self.rows.len().min(target_len);
        for (index, row) in self.rows.iter_mut().enumerate().take(reuse_len) {
            let variable = variables.get(index).unwrap_or(&blank);
            row.enabled = variable.enabled;
            row.secret = variable.secret;
            row.name.update(cx, |state, cx| {
                state.set_value(variable.name.clone(), window, cx)
            });
            row.value.update(cx, |state, cx| {
                state.set_value(variable.value.clone(), window, cx)
            });
        }
        self.rows.truncate(target_len);
        for index in reuse_len..target_len {
            let variable = variables.get(index).cloned().unwrap_or_else(blank_variable);
            self.push_row(variable, window, cx);
        }
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
        let header = table_header_row(
            border,
            theme.muted,
            theme.muted_foreground,
            true,
            true,
            vec![
                (
                    rust_i18n::t!("api.request.environment_name").to_string(),
                    ColumnWidth::Relative(0.32),
                ),
                (
                    rust_i18n::t!("api.request.environment_value").to_string(),
                    ColumnWidth::Flex,
                ),
                (
                    rust_i18n::t!("api.request.environment_secret").to_string(),
                    ColumnWidth::Fixed(rems(6.5)),
                ),
            ],
        );
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let add = cx.entity().downgrade();
            let enabled = cx.entity().downgrade();
            let secret = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            table_row_container("api-env-row", id, !blank && !row.enabled, border)
                .child(table_leading_toggle_slot(LeadingToggleSlotParams {
                    add_key: "api-env-add",
                    toggle_key: "api-env-enabled",
                    id,
                    blank,
                    enabled: row.enabled,
                    add_tooltip: rust_i18n::t!("api.request.add_environment").to_string(),
                    on_add: Box::new(move |_, window, cx| {
                        let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                    }),
                    on_toggle: Box::new(move |_, _, cx| {
                        let _ = enabled.update(cx, |this, cx| this.toggle_enabled(id, cx));
                    }),
                }))
                .child(table_input_cell(
                    &row.name,
                    ColumnWidth::Relative(0.32),
                    border,
                ))
                .child(table_input_cell(&row.value, ColumnWidth::Flex, border))
                .child(table_checkbox_cell(
                    "api-env-secret",
                    id,
                    row.secret,
                    ColumnWidth::Fixed(rems(6.5)),
                    border,
                    move |_, _, cx| {
                        let _ = secret.update(cx, |this, cx| this.toggle_secret(id, cx));
                    },
                ))
                .child(table_action_slot((!blank).then(|| {
                    table_remove_button(
                        "api-env-remove",
                        id,
                        rust_i18n::t!("api.request.remove_environment").to_string(),
                        move |_, window, cx| {
                            let _ = remove.update(cx, |this, cx| this.remove(id, window, cx));
                        },
                    )
                })))
                .into_any_element()
        });
        table_scroll_frame("api-environment-editor", border, header, rows)
    }
}
