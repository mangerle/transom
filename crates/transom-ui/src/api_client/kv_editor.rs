//! API 查询参数与请求头共用的三列表格编辑器。

use crate::theme::tokens::{size, space, typography};
use crate::widgets::IconButton;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, UiIcon};

/// 表格编辑器的业务类型，只影响占位文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KvEditorKind {
    Query,
    Header,
}

/// 表格中的一行值，事件携带完整快照，父实体无需回读编辑器。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KvEntry {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) description: String,
    pub(crate) enabled: bool,
}

/// 参数表变化事件。
pub(crate) enum KvEditorEvent {
    Changed(Vec<KvEntry>),
}

struct KvRow {
    id: u64,
    enabled: bool,
    name: Entity<InputState>,
    value: Entity<InputState>,
    description: Entity<InputState>,
    _subscriptions: [Subscription; 3],
}

/// 查询参数或请求头的三列表格实体。
pub(crate) struct KvEditor {
    kind: KvEditorKind,
    rows: Vec<KvRow>,
    next_row_id: u64,
    syncing: bool,
}

fn blank_entry() -> KvEntry {
    KvEntry {
        enabled: true,
        ..KvEntry::default()
    }
}

impl EventEmitter<KvEditorEvent> for KvEditor {}

impl KvEditor {
    /// 构造空表格；载入请求时再注入具体参数。
    pub(crate) fn new(kind: KvEditorKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            kind,
            rows: Vec::with_capacity(8),
            next_row_id: 0,
            syncing: false,
        };
        editor.push_row(blank_entry(), window, cx);
        editor
    }

    /// 用请求快照重建表格，并始终保留一个可继续输入的空白行。
    pub(crate) fn set_entries(
        &mut self,
        entries: &[KvEntry],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncing = true;
        self.rows.clear();
        for entry in entries {
            self.push_row(entry.clone(), window, cx);
        }
        self.push_row(blank_entry(), window, cx);
        self.syncing = false;
        cx.notify();
    }

    /// 读取非空行；空白行只负责继续添加，不进入请求模型。
    pub(crate) fn entries(&self, cx: &App) -> Vec<KvEntry> {
        self.rows
            .iter()
            .map(|row| KvEntry {
                name: row.name.read(cx).value().trim().to_string(),
                value: row.value.read(cx).value().to_string(),
                description: row.description.read(cx).value().trim().to_string(),
                enabled: row.enabled,
            })
            .filter(|entry| {
                !entry.name.is_empty() || !entry.value.is_empty() || !entry.description.is_empty()
            })
            .collect()
    }

    fn push_row(&mut self, entry: KvEntry, window: &mut Window, cx: &mut Context<Self>) {
        let (name_placeholder, value_placeholder, description_placeholder) = match self.kind {
            KvEditorKind::Query => (
                rust_i18n::t!("api.request.query_name_placeholder").to_string(),
                rust_i18n::t!("api.request.query_value_placeholder").to_string(),
                rust_i18n::t!("api.request.parameter_description").to_string(),
            ),
            KvEditorKind::Header => (
                rust_i18n::t!("api.request.header_name_placeholder").to_string(),
                rust_i18n::t!("api.request.query_value_placeholder").to_string(),
                rust_i18n::t!("api.request.parameter_description").to_string(),
            ),
        };
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(name_placeholder)
                .default_value(entry.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(value_placeholder)
                .default_value(entry.value)
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(description_placeholder)
                .default_value(entry.description)
        });
        let watch_name =
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            });
        let watch_value =
            cx.subscribe_in(&value, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            });
        let watch_description = cx.subscribe_in(
            &description,
            window,
            |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            },
        );
        self.next_row_id = self.next_row_id.saturating_add(1);
        self.rows.push(KvRow {
            id: self.next_row_id,
            enabled: entry.enabled,
            name,
            value,
            description,
            _subscriptions: [watch_name, watch_value, watch_description],
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
            self.push_row(blank_entry(), window, cx);
            return;
        };
        let empty = last.name.read(cx).value().is_empty()
            && last.value.read(cx).value().is_empty()
            && last.description.read(cx).value().is_empty();
        if !empty {
            self.push_row(blank_entry(), window, cx);
        }
    }

    fn report(&mut self, cx: &mut Context<Self>) {
        cx.emit(KvEditorEvent::Changed(self.entries(cx)));
        cx.notify();
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(blank_entry(), window, cx);
        self.report(cx);
    }

    fn toggle(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.enabled = !row.enabled;
            self.report(cx);
        }
    }

    fn remove(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }
}

impl Render for KvEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let last = self.rows.len().saturating_sub(1);
        let header = h_flex()
            .h(rems(size::PANE_HEADER_HEIGHT))
            .bg(theme.muted)
            .text_size(rems(typography::CAPTION))
            .text_color(theme.muted_foreground)
            .border_color(border)
            .border_b_1()
            .child(div().w(rems(2.0)).flex_none())
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .w(relative(0.28))
                            .flex_none()
                            .min_w_0()
                            .h_full()
                            .px(rems(space::SM))
                            .flex()
                            .items_center()
                            .border_l_1()
                            .border_color(border)
                            .child(rust_i18n::t!("api.request.parameter_name").to_string()),
                    )
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
                            .child(rust_i18n::t!("api.request.parameter_value").to_string()),
                    )
                    .child(
                        div()
                            .w(relative(0.28))
                            .flex_none()
                            .min_w_0()
                            .h_full()
                            .px(rems(space::SM))
                            .flex()
                            .items_center()
                            .border_l_1()
                            .border_color(border)
                            .child(rust_i18n::t!("api.request.parameter_description").to_string()),
                    ),
            )
            .child(div().w(rems(2.0)).flex_none());

        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let toggle = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            h_flex()
                .id(("api-kv-row", id))
                .h(rems(size::TREE_ROW_HEIGHT))
                .items_center()
                .border_color(border)
                .border_t_1()
                .when(!blank && !row.enabled, |row| row.opacity(0.45))
                .child(
                    div()
                        .w(rems(2.0))
                        .flex_none()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(blank, |cell| {
                            let add = cx.entity().downgrade();
                            cell.child(
                                IconButton::new(
                                    ("api-kv-add", id),
                                    AppIcon::Ui(UiIcon::ADD),
                                    rust_i18n::t!("api.request.add_parameter").to_string(),
                                )
                                .render()
                                .on_click(move |_, window, cx| {
                                    let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                                }),
                            )
                        })
                        .when(!blank, |cell| {
                            cell.child(
                                Checkbox::new(("api-kv-enabled", id))
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
                                .w(relative(0.28))
                                .flex_none()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
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
                                .border_l_1()
                                .border_color(border)
                                .child(Input::new(&row.value).small().appearance(false)),
                        )
                        .child(
                            div()
                                .w(relative(0.28))
                                .flex_none()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .border_l_1()
                                .border_color(border)
                                .child(Input::new(&row.description).small().appearance(false)),
                        ),
                )
                .child(
                    div()
                        .w(rems(2.0))
                        .flex_none()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!blank, |cell| {
                            cell.child(
                                IconButton::new(
                                    ("api-kv-remove", id),
                                    AppIcon::Ui(UiIcon::DELETE),
                                    rust_i18n::t!("api.request.remove_parameter").to_string(),
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
            .id("api-kv-editor")
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
