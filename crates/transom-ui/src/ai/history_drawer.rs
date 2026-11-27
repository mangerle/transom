//! AI 会话历史列表抽屉：展示按作用域归档的会话与自动生成标题，支持切换与删除。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::ConversationSummary;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::state::AiDrawerTab;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 将 RFC3339 时间戳格式化为简洁本地月日时分（解析失败时回退原串）。
pub(super) fn format_short_timestamp(raw: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return dt
            .with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string();
    }
    raw.to_owned()
}

/// 历史会话抽屉渲染参数。
pub(super) struct HistoryDrawerParams<'a> {
    pub(super) conversations: &'a [ConversationSummary],
    pub(super) active_id: Option<&'a str>,
    pub(super) show_archived: bool,
    pub(super) p: &'a Palette,
}

type HistoryIdCb = Rc<dyn Fn(String, &mut Window, &mut App)>;
type HistoryArchiveCb = Rc<dyn Fn(String, bool, &mut Window, &mut App)>;
type HistoryBoolCb = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// 历史会话抽屉交互回调。
pub(super) struct HistoryDrawerActions {
    pub(super) on_select: HistoryIdCb,
    pub(super) on_toggle_archive: HistoryArchiveCb,
    pub(super) on_delete: HistoryIdCb,
    pub(super) on_switch_archived: HistoryBoolCb,
}

/// 渲染历史会话列表抽屉（支持活跃/已归档切换与打开、归档、取消归档、删除）。
pub(super) fn render_history_drawer(
    params: HistoryDrawerParams<'_>,
    actions: HistoryDrawerActions,
) -> AnyElement {
    let HistoryDrawerParams {
        conversations,
        active_id,
        show_archived,
        p,
    } = params;
    let active_count = conversations.iter().filter(|c| !c.archived).count();
    let archived_count = conversations.len().saturating_sub(active_count);
    let filtered: Vec<&ConversationSummary> = conversations
        .iter()
        .filter(|c| c.archived == show_archived)
        .collect();

    let mut tabs = h_flex()
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(p.radius.md)
        .bg(p.secondary);
    for (is_arch_tab, label) in [
        (
            false,
            rust_i18n::t!("ai.history_tab_active", count = active_count).to_string(),
        ),
        (
            true,
            rust_i18n::t!("ai.history_tab_archived", count = archived_count).to_string(),
        ),
    ] {
        let on_sw = actions.on_switch_archived.clone();
        let active_tab = show_archived == is_arch_tab;
        let mut tab = h_flex()
            .id(if is_arch_tab {
                "ai-hist-tab-archived"
            } else {
                "ai-hist-tab-active"
            })
            .px(rems(space::XS))
            .py(px(3.0))
            .rounded(p.radius.sm)
            .cursor_pointer()
            .text_size(rems(typography::CAPTION))
            .on_click(move |_, w, cx| on_sw(is_arch_tab, w, cx))
            .child(label);
        tab = if active_tab {
            tab.bg(p.surface.floating)
                .shadow_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
        } else {
            tab.text_color(p.text.muted)
                .hover(|s| s.text_color(p.foreground))
        };
        tabs = tabs.child(tab);
    }

    let mut root = v_flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .overflow_y_scrollbar()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("ai.history_title").to_string()),
                )
                .child(tabs),
        );

    if filtered.is_empty() {
        let empty_text = if show_archived {
            rust_i18n::t!("ai.history_archived_empty").to_string()
        } else {
            rust_i18n::t!("ai.history_empty").to_string()
        };
        return root
            .child(
                div()
                    .p(rems(space::MD))
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(empty_text),
            )
            .into_any_element();
    }

    for (idx, conv) in filtered.into_iter().enumerate() {
        let is_active = active_id == Some(conv.id.as_str());
        let id_row = conv.id.clone();
        let id_open = conv.id.clone();
        let id_arch = conv.id.clone();
        let id_del = conv.id.clone();
        let next_arch = !conv.archived;
        let select_row = actions.on_select.clone();
        let select_btn = actions.on_select.clone();
        let toggle_arch = actions.on_toggle_archive.clone();
        let delete = actions.on_delete.clone();
        let open_tip = rust_i18n::t!("ai.history_open_tip").to_string();
        let arch_tip = if conv.archived {
            rust_i18n::t!("ai.history_unarchive_tip").to_string()
        } else {
            rust_i18n::t!("ai.history_archive_tip").to_string()
        };
        let arch_icon = if conv.archived {
            UiIcon::RESTORE
        } else {
            UiIcon::SHELVE
        };
        let del_tip = rust_i18n::t!("ai.delete_conversation").to_string();

        let mut row = h_flex()
            .id(("ai-hist-row", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .justify_between()
            .items_center()
            .rounded(p.radius.md)
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| select_row(id_row.clone(), w, cx));
        if is_active {
            row = row
                .bg(p.surface.selected)
                .border_l_2()
                .border_color(p.primary);
        }
        row = row
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(rems(space::XS))
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if is_active { p.primary } else { p.text.muted })
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::HISTORY)).small()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::BODY))
                                    .font_weight(if is_active {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(p.foreground)
                                    .child(conv.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(format_short_timestamp(&conv.updated_at)),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap(px(2.0))
                    .items_center()
                    .child(
                        Button::new(("ai-hist-open", idx))
                            .ghost()
                            .xsmall()
                            .accessibility_label(open_tip.clone())
                            .tooltip(open_tip)
                            .child(div().text_color(p.text.muted).child(
                                AppIconElement::new(AppIcon::Ui(UiIcon::EXTERNAL_LINK)).small(),
                            ))
                            .on_click(move |_, w, cx| {
                                cx.stop_propagation();
                                select_btn(id_open.clone(), w, cx);
                            }),
                    )
                    .child(
                        Button::new(("ai-hist-arch", idx))
                            .ghost()
                            .xsmall()
                            .accessibility_label(arch_tip.clone())
                            .tooltip(arch_tip)
                            .child(
                                div()
                                    .text_color(p.text.muted)
                                    .child(AppIconElement::new(AppIcon::Ui(arch_icon)).small()),
                            )
                            .on_click(move |_, w, cx| {
                                cx.stop_propagation();
                                toggle_arch(id_arch.clone(), next_arch, w, cx);
                            }),
                    )
                    .child(
                        Button::new(("ai-hist-del", idx))
                            .ghost()
                            .xsmall()
                            .accessibility_label(del_tip.clone())
                            .tooltip(del_tip)
                            .child(
                                div().text_color(p.text.muted).child(
                                    AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small(),
                                ),
                            )
                            .on_click(move |_, w, cx| {
                                cx.stop_propagation();
                                delete(id_del.clone(), w, cx);
                            }),
                    ),
            );
        root = root.child(row);
    }
    root.into_any_element()
}

impl AiAssistantView {
    pub(super) fn render_history_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let v_sel = cx.entity();
        let v_arch = cx.entity();
        let v_del = cx.entity();
        let v_sw = cx.entity();
        render_history_drawer(
            HistoryDrawerParams {
                conversations: &self.state.conversation.conversations,
                active_id: self.state.conversation.active_conversation_id.as_deref(),
                show_archived: self.state.conversation.show_archived_history,
                p,
            },
            HistoryDrawerActions {
                on_select: Rc::new(move |id, _, cx| {
                    v_sel.update(cx, |this, cx| {
                        this.select_conversation(&id);
                        this.state.view.drawer = AiDrawerTab::Chat;
                        cx.notify();
                    });
                }),
                on_toggle_archive: Rc::new(move |id, archived, _, cx| {
                    v_arch.update(cx, |this, cx| {
                        this.set_conversation_archived(&id, archived, cx);
                    });
                }),
                on_delete: Rc::new(move |id, _, cx| {
                    v_del.update(cx, |this, cx| this.delete_conversation(&id, cx));
                }),
                on_switch_archived: Rc::new(move |show_arch, _, cx| {
                    v_sw.update(cx, |this, cx| {
                        this.state.conversation.show_archived_history = show_arch;
                        cx.notify();
                    });
                }),
            },
        )
    }
}
