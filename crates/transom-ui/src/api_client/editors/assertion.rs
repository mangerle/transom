//! 请求断言逐行编辑器。

use super::table::{
    ColumnWidth, table_action_slot, table_header_row, table_input_cell, table_leading_toggle_slot,
    table_remove_button, table_row_container, table_scroll_frame,
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use transom_api::ApiAssertion;

pub(crate) enum AssertionEditorEvent {
    Changed(Vec<ApiAssertion>),
}

struct AssertionRow {
    id: u64,
    enabled: bool,
    expression: Entity<InputState>,
    _subscription: Subscription,
}

pub(crate) struct AssertionEditor {
    rows: Vec<AssertionRow>,
    next_id: u64,
    syncing: bool,
}

fn blank_assertion() -> ApiAssertion {
    ApiAssertion {
        expression: String::new(),
        enabled: true,
    }
}

impl EventEmitter<AssertionEditorEvent> for AssertionEditor {}

impl AssertionEditor {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            rows: Vec::with_capacity(8),
            next_id: 0,
            syncing: false,
        };
        editor.push_row(blank_assertion(), window, cx);
        editor
    }

    pub(crate) fn set_assertions(
        &mut self,
        assertions: &[ApiAssertion],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncing = true;
        self.rows.clear();
        for assertion in assertions {
            self.push_row(assertion.clone(), window, cx);
        }
        self.push_row(blank_assertion(), window, cx);
        self.syncing = false;
        cx.notify();
    }

    pub(crate) fn assertions(&self, cx: &App) -> Vec<ApiAssertion> {
        self.rows
            .iter()
            .map(|row| ApiAssertion {
                expression: row.expression.read(cx).value().trim().to_string(),
                enabled: row.enabled,
            })
            .filter(|assertion| !assertion.expression.is_empty())
            .collect()
    }

    fn push_row(&mut self, assertion: ApiAssertion, window: &mut Window, cx: &mut Context<Self>) {
        let expression = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.assertion_placeholder").to_string())
                .default_value(assertion.expression)
        });
        let subscription = cx.subscribe_in(
            &expression,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) && !this.syncing {
                    this.ensure_trailing_blank(window, cx);
                    this.report(cx);
                }
            },
        );
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(AssertionRow {
            id: self.next_id,
            enabled: assertion.enabled,
            expression,
            _subscription: subscription,
        });
    }

    fn ensure_trailing_blank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.rows.last() else {
            self.push_row(blank_assertion(), window, cx);
            return;
        };
        if !last.expression.read(cx).value().is_empty() {
            self.push_row(blank_assertion(), window, cx);
        }
    }

    fn report(&mut self, cx: &mut Context<Self>) {
        cx.emit(AssertionEditorEvent::Changed(self.assertions(cx)));
        cx.notify();
    }

    fn toggle(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.enabled = !row.enabled;
            self.report(cx);
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(blank_assertion(), window, cx);
        self.report(cx);
    }

    fn remove(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }
}

impl Render for AssertionEditor {
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
            vec![(
                rust_i18n::t!("api.request.assertion_expression").to_string(),
                ColumnWidth::Flex,
            )],
        );
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let add = cx.entity().downgrade();
            let toggle = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            table_row_container("api-assertion-row", id, !blank && !row.enabled, border)
                .child(table_leading_toggle_slot(
                    "api-assertion-add",
                    "api-assertion-enabled",
                    id,
                    blank,
                    row.enabled,
                    rust_i18n::t!("api.request.add_assertion").to_string(),
                    move |_, window, cx| {
                        let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                    },
                    move |_, _, cx| {
                        let _ = toggle.update(cx, |this, cx| this.toggle(id, cx));
                    },
                ))
                .child(table_input_cell(&row.expression, ColumnWidth::Flex, border))
                .child(table_action_slot((!blank).then(|| {
                    table_remove_button(
                        "api-assertion-remove",
                        id,
                        rust_i18n::t!("api.request.remove_assertion").to_string(),
                        move |_, window, cx| {
                            let _ = remove.update(cx, |this, cx| this.remove(id, window, cx));
                        },
                    )
                })))
                .into_any_element()
        });
        table_scroll_frame("api-assertion-editor", border, header, rows)
    }
}
