//! 常用快速命令条（Snippets）：底栏一键把长命令填进当前窗格，或直接发送执行。
//!
//! 与"广播/撰写窗格"（路线图 #10/#53/#54）是两件事：这里只有一个目标——当前焦点窗格，
//! 命令来自用户预先维护的库，不涉及多窗格分发，因此不引入任何目标选择器。

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_core::{Snippet, SnippetId};

/// 允许维护的命令条数上限：底栏是横向排列，再多就点不准了。
const SNIPPET_MAX_ITEMS: usize = 40;

/// 编辑弹窗里的一行命令；`id` 为 `None` 表示本次新增。
pub(crate) struct SnippetRow {
    pub(crate) id: Option<SnippetId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) command: Entity<InputState>,
}

/// 命令库编辑弹窗状态。
pub(crate) struct SnippetEditor {
    pub(crate) rows: Vec<SnippetRow>,
    pub(crate) error: Option<String>,
}

impl SnippetEditor {
    /// 取出各行的输入框实体交给元素树，避免渲染期间继续借用 `self`。
    fn row_inputs(&self) -> Vec<(Entity<InputState>, Entity<InputState>)> {
        self.rows
            .iter()
            .map(|row| (row.name.clone(), row.command.clone()))
            .collect()
    }
}

impl AppShell {
    /// 快捷键入口：展开/收起底栏。
    pub(crate) fn on_toggle_snippets(
        &mut self,
        _: &crate::keymap::ToggleSnippets,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_snippets_bar(cx);
    }

    /// 展开/收起底栏。展开状态属运行期（与侧边栏、沉浸模式同一口径），不落盘。
    pub(crate) fn toggle_snippets_bar(&mut self, cx: &mut Context<Self>) {
        self.snippets_expanded = !self.snippets_expanded;
        cx.notify();
    }

    /// 把一条命令写进当前窗格。
    ///
    /// 默认只填充不执行：终端里回车即执行，误点一次的代价可能是一条 `rm`；
    /// 需要"点一下就跑"时按住 Shift，这一点写在按钮 tooltip 里。
    pub(crate) fn apply_snippet(
        &mut self,
        snippet: &Snippet,
        execute: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.active_pane_terminal() else {
            self.status = Some(rust_i18n::t!("snippets.no_terminal").to_string());
            cx.notify();
            return;
        };
        let text = if execute {
            format!("{}\r", snippet.command)
        } else {
            snippet.command.clone()
        };
        let sent = terminal.update(cx, |view, cx| view.send_text(&text, cx));
        if !sent {
            self.error = Some(rust_i18n::t!("snippets.send_failed").to_string());
        } else if !execute {
            self.status = Some(rust_i18n::t!("snippets.filled").to_string());
        }
        cx.notify();
    }

    pub(crate) fn open_snippet_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.settings.snippets.items.clone();
        let rows = items
            .into_iter()
            .map(|item| self.new_snippet_row(Some(item), window, cx))
            .collect();
        self.snippet_editor = Some(SnippetEditor { rows, error: None });
        cx.notify();
    }

    pub(crate) fn close_snippet_editor(&mut self, cx: &mut Context<Self>) {
        self.snippet_editor = None;
        cx.notify();
    }

    fn new_snippet_row(
        &mut self,
        existing: Option<Snippet>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SnippetRow {
        let (id, name, command) = match existing {
            Some(item) => (Some(item.id), item.name, item.command),
            None => (None, String::new(), String::new()),
        };
        let name_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("snippets.name_hint").to_string())
        });
        name_input.update(cx, |state, cx| state.set_value(name, window, cx));
        let command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("snippets.command_hint").to_string())
        });
        command_input.update(cx, |state, cx| state.set_value(command, window, cx));
        SnippetRow {
            id,
            name: name_input,
            command: command_input,
        }
    }

    pub(crate) fn add_snippet_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let limit_reached = self
            .snippet_editor
            .as_ref()
            .is_some_and(|editor| editor.rows.len() >= SNIPPET_MAX_ITEMS);
        if limit_reached {
            if let Some(editor) = self.snippet_editor.as_mut() {
                editor.error = Some(rust_i18n::t!("snippets.too_many").to_string());
            }
            cx.notify();
            return;
        }
        let row = self.new_snippet_row(None, window, cx);
        if let Some(editor) = self.snippet_editor.as_mut() {
            editor.rows.push(row);
            editor.error = None;
        }
        cx.notify();
    }

    pub(crate) fn remove_snippet_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self.snippet_editor.as_mut()
            && index < editor.rows.len()
        {
            editor.rows.remove(index);
            cx.notify();
        }
    }

    /// 校验并保存命令库：任一行不合法就整体不保存，并指出问题在第几行。
    ///
    /// 逐行拒绝而不是跳过坏行，是因为"保存了但少一条"最难被发现。
    pub(crate) fn save_snippet_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.snippet_editor.as_ref() else {
            return;
        };
        let mut items = Vec::with_capacity(editor.rows.len());
        let mut invalid_row = None;
        for (index, row) in editor.rows.iter().enumerate() {
            let name = row.name.read(cx).value().trim().to_string();
            let command = row.command.read(cx).value().trim().to_string();
            let candidate = Snippet {
                id: row.id.unwrap_or_else(SnippetId::new),
                name,
                command,
            };
            if !candidate.is_valid() {
                invalid_row = Some(index + 1);
                break;
            }
            items.push(candidate);
        }
        if let Some(row_number) = invalid_row {
            if let Some(editor) = self.snippet_editor.as_mut() {
                editor.error = Some(
                    rust_i18n::t!("snippets.row_invalid", row = row_number.to_string()).to_string(),
                );
            }
            cx.notify();
            return;
        }
        self.settings.snippets.items = items;
        self.persist_snippets(cx);
    }

    fn persist_snippets(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
            cx.notify();
            return;
        }
        self.snippet_editor = None;
        self.status = Some(rust_i18n::t!("snippets.saved").to_string());
        cx.notify();
    }

    /// 渲染底部快捷命令条。
    pub(crate) fn render_snippets_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let items = self.settings.snippets.items.clone();
        let empty_hint = items.is_empty();
        let buttons = items.into_iter().map(|item| {
            let entity = cx.entity().clone();
            Button::new(ElementId::Name(format!("snippet-{}", item.id).into()))
                .ghost()
                .xsmall()
                .label(item.name.clone())
                .tooltip(item.command.clone())
                .on_click(move |event, _, cx| {
                    let execute = event.modifiers().shift;
                    let snippet = item.clone();
                    entity.update(cx, |this, cx| this.apply_snippet(&snippet, execute, cx));
                })
        });

        let mut row = h_flex()
            .id("snippets-bar")
            .h(px(32.0))
            .flex_none()
            .px_2()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(p.border)
            .bg(p.status_bar)
            .child(
                div()
                    .text_xs()
                    .flex_none()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("snippets.title").to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scrollbar()
                    .child(if empty_hint {
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("snippets.empty").to_string())
                            .into_any_element()
                    } else {
                        h_flex()
                            .gap_1()
                            .items_center()
                            .children(buttons)
                            .into_any_element()
                    }),
            )
            .child(
                Button::new("snippets-manage")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ListChecks)
                    .label(rust_i18n::t!("snippets.manage").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_snippet_editor(window, cx);
                    })),
            )
            .child(
                Button::new("snippets-collapse")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip(rust_i18n::t!("snippets.collapse").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_snippets_bar(cx);
                    })),
            );
        if empty_hint {
            // 空库时把"新建"放在栏里，否则底栏只有一句提示，用户还得再去点管理。
            row = row.child(
                Button::new("snippets-first-add")
                    .link()
                    .xsmall()
                    .label(rust_i18n::t!("snippets.add").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_snippet_editor(window, cx);
                    })),
            );
        }
        row.into_any_element()
    }

    pub(crate) fn render_snippet_editor(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self
            .snippet_editor
            .as_ref()
            .map(|editor| editor.row_inputs())
            .unwrap_or_default();
        let error = self
            .snippet_editor
            .as_ref()
            .and_then(|editor| editor.error.clone());
        let remove_label = rust_i18n::t!("common.delete").to_string();
        let name_column = rust_i18n::t!("snippets.name_column").to_string();
        let command_column = rust_i18n::t!("snippets.command_column").to_string();

        let rows_element = v_flex().gap_2().children(rows.into_iter().enumerate().map(
            |(index, (name, command))| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(Input::new(&name).w_full()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&command).w_full()),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("snippet-remove-{index}").into()))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .label(remove_label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_snippet_row(index, cx);
                            })),
                    )
                    .into_any_element()
            },
        ));

        let panel = v_flex()
            .w(px(760.0))
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.popover)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("snippets.editor_title").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("snippets.editor_note").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .px_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(name_column),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(command_column),
                    )
                    .child(div().w(px(72.0))),
            )
            .child(
                div()
                    .id("snippet-rows")
                    .max_h(px(360.0))
                    .overflow_y_scroll()
                    .child(rows_element),
            )
            .children(error.map(|message| div().text_xs().text_color(p.danger).child(message)))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .border_t_1()
                    .border_color(p.border)
                    .pt_2()
                    .child(
                        Button::new("snippet-add-row")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("snippets.add").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_snippet_row(window, cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("snippet-cancel")
                                    .ghost()
                                    .small()
                                    .label(rust_i18n::t!("common.cancel").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_snippet_editor(cx);
                                    })),
                            )
                            .child(
                                Button::new("snippet-save")
                                    .primary()
                                    .small()
                                    .label(rust_i18n::t!("common.save").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_snippet_editor(cx);
                                    })),
                            ),
                    ),
            );

        Self::overlay(panel, p)
    }
}
