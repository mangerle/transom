//! 快捷命令（Snippets）：底栏图标向上弹出卡片，把长命令填进当前窗格或直接执行。
//!
//! 与"广播/撰写窗格"（路线图 #10/#53/#54）是两件事：这里只有一个目标——当前焦点窗格，
//! 命令来自用户预先维护的库，不涉及多窗格分发，因此不引入任何目标选择器。
//!
//! 卡片显隐属运行期状态；"直接执行"跨重启保存，因为它是用户对风险的选择，
//! 但默认关闭——终端里误执行一条命令的代价远高于多点一次回车。

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::*;
use transom_core::{Snippet, SnippetId};

/// 卡片里命令预览的单行长度上限，避免一条超长命令把卡片撑破。
const SNIPPET_PREVIEW_MAX_CHARS: usize = 160;

/// 允许维护的命令条数上限：卡片是竖向列表，再多滚动查找也会变得困难。
const SNIPPET_MAX_ITEMS: usize = 40;

/// 新增/编辑单条快捷命令的小弹窗状态；`id` 为 `None` 表示新增。
pub(crate) struct SnippetFormState {
    id: Option<SnippetId>,
    name: Entity<InputState>,
    command: Entity<InputState>,
    error: Option<String>,
}

/// 底栏快捷命令卡片的显隐与编辑状态。
pub(crate) struct SnippetPanelState {
    pub(crate) open: bool,
    pub(crate) form: Option<SnippetFormState>,
}

impl AppShell {
    /// 快捷键入口：打开/关闭快捷命令卡片。
    pub(crate) fn on_toggle_snippets(
        &mut self,
        _: &crate::keymap::ToggleSnippets,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_snippets_panel(cx);
    }

    /// 打开/关闭快捷命令卡片。显隐属运行期（与侧边栏、沉浸模式同一口径），不落盘。
    ///
    /// 沉浸模式没有底栏，卡片失去锚点，因此直接忽略；已在沉浸模式时打开的卡片
    /// 会在进入沉浸的那一刻关闭。
    pub(crate) fn toggle_snippets_panel(&mut self, cx: &mut Context<Self>) {
        if self.immersive {
            return;
        }
        self.modals.snippet_panel.open = !self.modals.snippet_panel.open;
        cx.notify();
    }

    /// 把一条命令写进当前窗格。
    ///
    /// 是否直接执行由卡片底部的开关决定；默认只填充，终端里误执行一条
    /// 危险命令的代价远高于多点一次回车。
    pub(crate) fn apply_snippet(
        &mut self,
        snippet: &Snippet,
        execute: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.active_pane_terminal() else {
            // 徽标在场时状态文字没有显示位置，这里改用错误通知，避免点了没反应。
            self.set_error(rust_i18n::t!("snippets.no_terminal").to_string());
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
            self.set_error(rust_i18n::t!("snippets.send_failed").to_string());
        } else if !execute {
            self.status = Some(rust_i18n::t!("snippets.filled").to_string());
        }
        cx.notify();
    }

    /// 打开新增/编辑单条命令的小弹窗；`existing` 为 `None` 表示新增。
    ///
    /// 命令库不再提供多行管理面板：卡片行内已经有编辑与删除，新增和编辑
    /// 只处理一条命令。新增超过条数上限时直接提示，不打开弹窗。
    pub(crate) fn open_snippet_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        existing: Option<Snippet>,
    ) {
        if existing.is_none() && self.settings.snippets.items.len() >= SNIPPET_MAX_ITEMS {
            self.set_error(rust_i18n::t!("snippets.too_many").to_string());
            cx.notify();
            return;
        }
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
        let focus = name_input.read(cx).focus_handle(cx);
        self.modals.snippet_panel.form = Some(SnippetFormState {
            id,
            name: name_input,
            command: command_input,
            error: None,
        });
        // 卡片先收起：编辑弹窗是独立浮层，不应该和卡片叠在一起。
        self.modals.snippet_panel.open = false;
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn close_snippet_form(&mut self, cx: &mut Context<Self>) {
        self.modals.snippet_panel.form = None;
        cx.notify();
    }

    /// 校验并保存单条命令。名称或命令不合法时保留弹窗并就地提示。
    pub(crate) fn save_snippet_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.modals.snippet_panel.form.as_ref() else {
            return;
        };
        let id = form.id;
        let name = form.name.read(cx).value().trim().to_string();
        let command = form.command.read(cx).value().trim().to_string();
        let candidate = Snippet {
            id: id.unwrap_or_else(SnippetId::new),
            name,
            command,
        };
        if !candidate.is_valid() {
            if let Some(form) = self.modals.snippet_panel.form.as_mut() {
                form.error = Some(rust_i18n::t!("snippets.form_invalid").to_string());
            }
            cx.notify();
            return;
        }
        if id.is_none() && self.settings.snippets.items.len() >= SNIPPET_MAX_ITEMS {
            if let Some(form) = self.modals.snippet_panel.form.as_mut() {
                form.error = Some(rust_i18n::t!("snippets.too_many").to_string());
            }
            cx.notify();
            return;
        }
        if let Some(existing) = self
            .settings
            .snippets
            .items
            .iter_mut()
            .find(|item| item.id == candidate.id)
        {
            *existing = candidate;
        } else {
            self.settings.snippets.items.push(candidate);
        }
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            if let Some(form) = self.modals.snippet_panel.form.as_mut() {
                form.error = Some(localized_error(&error));
            }
            cx.notify();
            return;
        }
        self.modals.snippet_panel.form = None;
        self.status = Some(rust_i18n::t!("snippets.saved").to_string());
        cx.notify();
    }

    /// 从卡片里删除一条命令并立即落盘；卡片保持打开，便于连续清理。
    fn delete_snippet(&mut self, id: SnippetId, cx: &mut Context<Self>) {
        let before = self.settings.snippets.items.len();
        self.settings.snippets.items.retain(|item| item.id != id);
        if self.settings.snippets.items.len() != before {
            self.persist_layout_settings(cx);
        }
    }

    /// 渲染底栏里的快捷命令入口：图标按钮加向上弹出的卡片。
    ///
    /// 显隐由 `snippet_panel.open` 控制，因此底栏图标、活动栏条目与快捷键打开的是
    /// 同一个卡片；`Popover` 负责向上定位、点外部与 Esc 关闭。
    pub(crate) fn render_snippets_popover(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.modals.snippet_panel.open;
        let items = self.settings.snippets.items.clone();
        let execute_directly = self.settings.snippets.execute_directly;
        let app = cx.entity().clone();
        let trigger = Button::new("snippets-trigger")
            .ghost()
            .xsmall()
            .icon(IconName::ListChecks)
            .selected(open)
            .tooltip(rust_i18n::t!("snippets.title").to_string());

        let app_open = app.clone();
        let card_app = app;
        let card_items = items;
        let card_palette = p.clone();
        // 锚点直接用底栏高度而不是百分比：卡片底边与底栏上缘对齐，不跟着
        // 居中的按钮留出一条缝。左内边距让图标离分界线有一点呼吸，但触发器
        // 容器本身仍从分界线起算，卡片左缘才能贴住会话列表与终端的分界。
        let trigger_style = StyleRefinement::default()
            .h(rems(size::STATUS_BAR_HEIGHT))
            .flex()
            .flex_none()
            .items_center()
            .pl(px(6.0));
        Popover::new("snippets-popover")
            .anchor(Anchor::BottomLeft)
            // 向底栏压 1px：盖住底栏上边框，避免圆角与阴影之间露出细缝。
            .offset(px(-1.0))
            .trigger_style(trigger_style)
            .open(open)
            .on_open_change(move |open, _window, cx| {
                let open = *open;
                app_open.update(cx, |this, cx| {
                    if this.modals.snippet_panel.open != open {
                        this.modals.snippet_panel.open = open;
                        cx.notify();
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, _, _| {
                render_snippets_card(&card_app, &card_items, execute_directly, &card_palette)
            })
            .into_any_element()
    }

    /// 渲染新增/编辑单条快捷命令的小弹窗。
    pub(crate) fn render_snippet_form(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(form) = self.modals.snippet_panel.form.as_ref() else {
            return div().into_any_element();
        };
        let title = if form.id.is_some() {
            rust_i18n::t!("snippets.form_edit_title").to_string()
        } else {
            rust_i18n::t!("snippets.form_add_title").to_string()
        };
        let name_input = form.name.clone();
        let command_input = form.command.clone();
        let error = form.error.clone();
        let content = v_flex()
            .gap_3()
            .child(Self::form_field(
                rust_i18n::t!("snippets.name_column").to_string(),
                name_input,
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("snippets.command_column").to_string(),
                command_input,
                p,
            ))
            .children(error.map(|message| div().text_xs().text_color(p.danger).child(message)));
        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("snippet-form-cancel")
                    .ghost()
                    .small()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.close_snippet_form(cx))),
            )
            .child(
                Button::new("snippet-form-save")
                    .primary()
                    .small()
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.save_snippet_form(cx))),
            );
        Self::overlay(
            Self::dialog_frame(
                "snippet-form",
                title,
                None,
                content,
                Some(footer),
                px(420.0),
                p,
            ),
            p,
        )
    }
}

mod card;
#[cfg(test)]
mod tests;

use card::render_snippets_card;
#[cfg(test)]
use card::snippet_preview;
