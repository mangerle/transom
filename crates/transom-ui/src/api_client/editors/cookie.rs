//! 项目 Cookie 属性表格编辑器。

use super::table::{
    ColumnWidth, table_action_slot, table_add_button, table_checkbox_cell, table_header_row,
    table_input_cell, table_remove_button, table_row_container, table_scroll_frame,
};
use crate::widgets::status_dot_bare;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use transom_api::ApiCookie;

pub(crate) enum CookieEditorEvent {
    Changed(Vec<ApiCookie>),
}

struct CookieRow {
    id: u64,
    name: Entity<InputState>,
    value: Entity<InputState>,
    domain: Entity<InputState>,
    path: Entity<InputState>,
    secure: bool,
    http_only: bool,
    expires_at: Option<u64>,
    _subscriptions: [Subscription; 4],
}

pub(crate) struct CookieEditor {
    rows: Vec<CookieRow>,
    next_id: u64,
    syncing: bool,
}

fn blank_cookie() -> ApiCookie {
    ApiCookie {
        name: String::new(),
        value: String::new(),
        domain: None,
        path: "/".to_string(),
        secure: false,
        http_only: false,
        expires_at: None,
    }
}

impl EventEmitter<CookieEditorEvent> for CookieEditor {}

impl CookieEditor {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            rows: Vec::with_capacity(8),
            next_id: 0,
            syncing: false,
        };
        editor.push_row(blank_cookie(), window, cx);
        editor
    }

    pub(crate) fn set_cookies(
        &mut self,
        cookies: &[ApiCookie],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syncing = true;
        self.rows.clear();
        for cookie in cookies {
            self.push_row(cookie.clone(), window, cx);
        }
        self.push_row(blank_cookie(), window, cx);
        self.syncing = false;
        cx.notify();
    }

    pub(crate) fn cookies(&self, cx: &App) -> Vec<ApiCookie> {
        self.rows
            .iter()
            .map(|row| ApiCookie {
                name: row.name.read(cx).value().trim().to_string(),
                value: row.value.read(cx).value().to_string(),
                domain: {
                    let domain = row.domain.read(cx).value().trim().to_string();
                    (!domain.is_empty()).then_some(domain)
                },
                path: row.path.read(cx).value().trim().to_string(),
                secure: row.secure,
                http_only: row.http_only,
                expires_at: row.expires_at,
            })
            .filter(|cookie| !cookie.name.is_empty() || !cookie.value.is_empty())
            .collect()
    }

    fn push_row(&mut self, cookie: ApiCookie, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.cookie_name_placeholder").to_string())
                .default_value(cookie.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.cookie_value_placeholder").to_string())
                .default_value(cookie.value)
        });
        let domain = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.cookie_domain_placeholder").to_string())
                .default_value(cookie.domain.unwrap_or_default())
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.cookie_path_placeholder").to_string())
                .default_value(cookie.path)
        });
        let mut watch = |input: &Entity<InputState>| {
            cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
                this.on_input(event, window, cx);
            })
        };
        let watch_name = watch(&name);
        let watch_value = watch(&value);
        let watch_domain = watch(&domain);
        let watch_path = watch(&path);
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(CookieRow {
            id: self.next_id,
            name,
            value,
            domain,
            path,
            secure: cookie.secure,
            http_only: cookie.http_only,
            expires_at: cookie.expires_at,
            _subscriptions: [watch_name, watch_value, watch_domain, watch_path],
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
            self.push_row(blank_cookie(), window, cx);
            return;
        };
        let empty = last.name.read(cx).value().is_empty()
            && last.value.read(cx).value().is_empty()
            && last.domain.read(cx).value().is_empty();
        if !empty {
            self.push_row(blank_cookie(), window, cx);
        }
    }

    fn report(&mut self, cx: &mut Context<Self>) {
        cx.emit(CookieEditorEvent::Changed(self.cookies(cx)));
        cx.notify();
    }

    fn toggle_secure(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.secure = !row.secure;
            self.report(cx);
        }
    }

    fn toggle_http_only(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.http_only = !row.http_only;
            self.report(cx);
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(blank_cookie(), window, cx);
        self.report(cx);
    }

    fn remove(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        self.ensure_trailing_blank(window, cx);
        self.report(cx);
    }
}

impl Render for CookieEditor {
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
                    rust_i18n::t!("api.request.cookie_name").to_string(),
                    ColumnWidth::Relative(0.2),
                ),
                (
                    rust_i18n::t!("api.request.cookie_value").to_string(),
                    ColumnWidth::Flex,
                ),
                (
                    rust_i18n::t!("api.request.cookie_domain").to_string(),
                    ColumnWidth::Relative(0.22),
                ),
                (
                    rust_i18n::t!("api.request.cookie_path").to_string(),
                    ColumnWidth::Fixed(rems(7.0)),
                ),
                (
                    rust_i18n::t!("api.request.cookie_secure").to_string(),
                    ColumnWidth::Fixed(rems(5.0)),
                ),
                (
                    rust_i18n::t!("api.request.cookie_http_only").to_string(),
                    ColumnWidth::Fixed(rems(5.5)),
                ),
            ],
        );
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let blank = index == last;
            let id = row.id;
            let add = cx.entity().downgrade();
            let secure = cx.entity().downgrade();
            let http_only = cx.entity().downgrade();
            let remove = cx.entity().downgrade();
            let leading = if blank {
                table_add_button(
                    "api-cookie-add",
                    id,
                    rust_i18n::t!("api.request.add_cookie").to_string(),
                    move |_, window, cx| {
                        let _ = add.update(cx, |this, cx| this.add_row(window, cx));
                    },
                )
            } else {
                status_dot_bare(theme.muted_foreground).into_any_element()
            };
            table_row_container("api-cookie-row", id, false, border)
                .child(table_action_slot(Some(leading)))
                .child(table_input_cell(
                    &row.name,
                    ColumnWidth::Relative(0.2),
                    border,
                ))
                .child(table_input_cell(&row.value, ColumnWidth::Flex, border))
                .child(table_input_cell(
                    &row.domain,
                    ColumnWidth::Relative(0.22),
                    border,
                ))
                .child(table_input_cell(
                    &row.path,
                    ColumnWidth::Fixed(rems(7.0)),
                    border,
                ))
                .child(table_checkbox_cell(
                    "api-cookie-secure",
                    id,
                    row.secure,
                    ColumnWidth::Fixed(rems(5.0)),
                    border,
                    move |_, _, cx| {
                        let _ = secure.update(cx, |this, cx| this.toggle_secure(id, cx));
                    },
                ))
                .child(table_checkbox_cell(
                    "api-cookie-http-only",
                    id,
                    row.http_only,
                    ColumnWidth::Fixed(rems(5.5)),
                    border,
                    move |_, _, cx| {
                        let _ = http_only.update(cx, |this, cx| this.toggle_http_only(id, cx));
                    },
                ))
                .child(table_action_slot((!blank).then(|| {
                    table_remove_button(
                        "api-cookie-remove",
                        id,
                        rust_i18n::t!("api.request.remove_cookie").to_string(),
                        move |_, window, cx| {
                            let _ = remove.update(cx, |this, cx| this.remove(id, window, cx));
                        },
                    )
                })))
                .into_any_element()
        });
        table_scroll_frame("api-cookie-editor", border, header, rows)
    }
}
