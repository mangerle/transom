//! 请求断言逐行编辑器。

use crate::theme::tokens::{size, space, typography};
use crate::widgets::IconButton;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::ApiAssertion;
use transom_ui_icons::{AppIcon, UiIcon};

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
        let header = h_flex()
            .h(rems(size::LARGE_HEADER_HEIGHT))
            .bg(theme.muted)
            .text_size(rems(typography::CAPTION))
            .text_color(theme.muted_foreground)
            .border_color(border)
            .border_b_1()
            .child(div().w(rems(3.0)).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(rems(space::SM))
                    .flex()
                    .items_center()
                    .border_l_1()
                    .border_color(border)
                    .child(rust_i18n::t!("api.request.assertion_expression").to_string()),
            )
            .child(div().w(rems(3.0)).flex_none());
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let toggle = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            h_flex()
                .id(("api-assertion-row", id))
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
                                    ("api-assertion-add", id),
                                    AppIcon::Ui(UiIcon::ADD),
                                    rust_i18n::t!("api.request.add_assertion").to_string(),
                                )
                                .render()
                                .on_click(move |_, window, cx| {
                                    let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                                }),
                            )
                        })
                        .when(!blank, |cell| {
                            cell.child(
                                Checkbox::new(("api-assertion-enabled", id))
                                    .checked(row.enabled)
                                    .on_click(move |_, _, cx| {
                                        let _ = toggle.update(cx, |this, cx| this.toggle(id, cx));
                                    }),
                            )
                        }),
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
                        .child(Input::new(&row.expression).small().appearance(false)),
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
                                    ("api-assertion-remove", id),
                                    AppIcon::Ui(UiIcon::DELETE),
                                    rust_i18n::t!("api.request.remove_assertion").to_string(),
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
            .id("api-assertion-editor")
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
