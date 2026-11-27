//! AI 会话历史列表抽屉：展示按作用域归档的会话与自动生成标题，支持切换与删除。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::{ConversationSummary, ThinkingEffort, default_acp_presets};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::config_drawer::{
    ConfigDrawerActions, ConfigDrawerParams, DraftModelRow, render_config_drawer,
};
use super::state::{AiActivePopup, AiDrawerTab};
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

/// 历史会话抽屉交互回调。
pub(super) struct HistoryDrawerActions {
    pub(super) on_select: HistoryIdCb,
    pub(super) on_toggle_archive: Rc<dyn Fn(String, bool, &mut Window, &mut App)>,
    pub(super) on_delete: HistoryIdCb,
    pub(super) on_switch_archived: Rc<dyn Fn(bool, &mut Window, &mut App)>,
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
        (false, format!("历史 ({active_count})")),
        (true, format!("已归档 ({archived_count})")),
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
            "暂无已归档会话".to_owned()
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
        let open_tip = "打开会话".to_owned();
        let arch_tip = if conv.archived {
            "取消归档".to_owned()
        } else {
            "归档会话".to_owned()
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
                            .child(
                                div()
                                    .text_color(p.text.muted)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::EXTERNAL_LINK)).small()),
                            )
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
                                div()
                                    .text_color(p.text.muted)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small()),
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

pub(super) fn make_draft_model_row(
    display_name: &str,
    request_model: &str,
    ctx_win: Option<u32>,
    effort: Option<ThinkingEffort>,
    window: &mut Window,
    cx: &mut App,
) -> DraftModelRow {
    let disp_val = display_name.to_owned();
    let req_val = request_model.to_owned();
    let ctx_val = ctx_win.unwrap_or(128_000).to_string();
    let display_input = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(disp_val)
            .placeholder(rust_i18n::t!("ai.cfg_display_placeholder").to_string())
    });
    let request_input = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(req_val)
            .placeholder(rust_i18n::t!("ai.cfg_request_placeholder").to_string())
    });
    let ctx_input = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(ctx_val)
            .placeholder("128000")
    });
    DraftModelRow {
        display_name: display_input,
        request_model: request_input,
        context_window: ctx_input,
        effort,
    }
}

impl AiAssistantView {
    pub(super) fn add_empty_draft_model_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.draft_models.push(make_draft_model_row(
            "",
            "",
            Some(128_000),
            None,
            window,
            cx,
        ));
        cx.notify();
    }

    pub(super) fn remove_draft_model_row(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.draft_models.len() {
            self.draft_models.remove(idx);
            cx.notify();
        }
    }

    pub(super) fn load_provider_into_form(
        &mut self,
        provider_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prov) = self
            .state
            .settings
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .cloned()
        else {
            return;
        };
        self.state.selected_protocol = prov.protocol;
        self.provider_name_input
            .update(cx, |i, cx| i.set_value(prov.name, window, cx));
        self.base_url_input
            .update(cx, |i, cx| i.set_value(prov.base_url, window, cx));
        let saved_key = self
            .ai_store
            .load_provider_api_key(self.secret_store.as_ref(), &prov.id)
            .ok()
            .flatten()
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.api_key_input
            .update(cx, |i, cx| i.set_value(saved_key, window, cx));
        self.draft_models = prov
            .models
            .iter()
            .map(|m| {
                make_draft_model_row(
                    &m.display_name,
                    &m.name,
                    m.context_window,
                    m.efforts.first().copied(),
                    window,
                    cx,
                )
            })
            .collect();
        if self.draft_models.is_empty() {
            self.draft_models.push(make_draft_model_row(
                "",
                "",
                Some(128_000),
                None,
                window,
                cx,
            ));
        }
        cx.notify();
    }
    pub(super) fn render_history_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let v_sel = cx.entity();
        let v_arch = cx.entity();
        let v_del = cx.entity();
        let v_sw = cx.entity();
        render_history_drawer(
            HistoryDrawerParams {
                conversations: &self.state.conversations,
                active_id: self.state.active_conversation_id.as_deref(),
                show_archived: self.state.show_archived_history,
                p,
            },
            HistoryDrawerActions {
                on_select: Rc::new(move |id, _, cx| {
                    v_sel.update(cx, |this, cx| {
                        this.select_conversation(&id);
                        this.state.drawer = AiDrawerTab::Chat;
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
                        this.state.show_archived_history = show_arch;
                        cx.notify();
                    });
                }),
            },
        )
    }

    pub(super) fn render_config_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let models_section = self.render_models_table_section(p, cx);
        let picker_panel = self.render_model_picker_panel(p, cx);
        let policy_hooks_section =
            super::policy_hooks_drawer::render_policy_hooks_section(self, p, cx);
        let view = cx.entity();
        let view_sec = view.clone();
        let view_test = view.clone();
        let view_fetch = view.clone();
        let view_picker = view.clone();
        let view_add_row = view.clone();
        let view_save = view.clone();
        let view_edit = view.clone();
        let view_del = view.clone();
        let view_acp_preset = view.clone();
        let view_acp_save = view.clone();
        let view_acp_install = view.clone();
        let selected_acp_idx = self.selected_acp_idx;
        render_config_drawer(
            ConfigDrawerParams {
                active_section: self.state.config_section,
                settings: &self.state.settings,
                selected_protocol: self.state.selected_protocol,
                provider_name_input: &self.provider_name_input,
                api_key_input: &self.api_key_input,
                base_url_input: &self.base_url_input,
                models_section,
                picker_panel,
                has_discovered_models: !self.state.discovered_models.is_empty(),
                acp_cmd_input: &self.acp_cmd_input,
                selected_acp_idx,
                acp_install_status: self.state.acp_install_status.as_ref(),
                discovery_status: self.state.discovery_status.as_deref(),
                lang_select: &self.lang_select,
                running_behavior_select: &self.running_behavior_select,
                policy_hooks_section,
                p,
            },
            ConfigDrawerActions {
                on_select_section: Rc::new(move |section, _, cx| {
                    view_sec.update(cx, |this, cx| {
                        this.state.config_section = section;
                        cx.notify();
                    });
                }),
                on_select_protocol: Rc::new(move |proto, _, cx| {
                    view.update(cx, |this, cx| {
                        this.state.selected_protocol = proto;
                        cx.notify();
                    });
                }),
                on_test_connection: Box::new(move |_, cx| {
                    view_test.update(cx, |this, cx| this.test_provider_connection(cx));
                }),
                on_fetch_models: Box::new(move |_, cx| {
                    view_fetch.update(cx, |this, cx| this.discover_provider_models(cx));
                }),
                on_open_model_picker: Box::new(move |_, cx| {
                    view_picker.update(cx, |this, cx| this.open_model_picker(cx));
                }),
                on_add_model_row: Box::new(move |w, cx| {
                    view_add_row.update(cx, |this, cx| this.add_empty_draft_model_row(w, cx));
                }),
                on_save_provider: Box::new(move |w, cx| {
                    view_save.update(cx, |this, cx| this.save_provider_from_drawer(w, cx));
                }),
                on_edit_provider: Rc::new(move |id, w, cx| {
                    view_edit.update(cx, |this, cx| this.load_provider_into_form(&id, w, cx));
                }),
                on_delete_provider: Rc::new(move |id, _, cx| {
                    view_del.update(cx, |this, cx| this.delete_provider(&id, cx));
                }),
                on_apply_acp_preset: Rc::new(move |idx, w, cx| {
                    view_acp_preset.update(cx, |this, cx| this.apply_acp_preset(idx, w, cx));
                }),
                on_save_acp: Box::new(move |w, cx| {
                    view_acp_save.update(cx, |this, cx| this.save_acp_from_drawer(w, cx));
                }),
                on_install_acp: Rc::new(move |id, _, cx| {
                    view_acp_install.update(cx, |this, cx| this.install_acp_agent(id, cx));
                }),
            },
        )
    }

    pub(super) fn apply_acp_preset(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let presets = default_acp_presets();
        let Some(agent) = presets.get(idx) else {
            return;
        };
        self.selected_acp_idx = idx;
        let line = if agent.args.is_empty() {
            agent.command.clone()
        } else {
            format!("{} {}", agent.command, agent.args.join(" "))
        };
        self.acp_cmd_input
            .update(cx, |i, cx| i.set_value(line, window, cx));
        cx.notify();
    }

    pub(super) fn save_acp_from_drawer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let presets = default_acp_presets();
        let base = presets
            .get(self.selected_acp_idx)
            .cloned()
            .unwrap_or_default();
        let raw_cmd = self.acp_cmd_input.read(cx).value().to_string();
        let parts: Vec<String> = raw_cmd.split_whitespace().map(str::to_owned).collect();
        let Some(cmd) = parts.first().cloned() else {
            return;
        };
        let mut agent = base;
        agent.command = cmd;
        agent.args = parts.into_iter().skip(1).collect();
        agent.enabled = true;
        let agent_id = agent.id.clone();
        self.state.settings.acp_agents.retain(|a| a.id != agent_id);
        self.state.settings.acp_agents.push(agent);
        self.state.settings.active_acp_agent_id = Some(agent_id.clone());
        self.state.active_acp_agent_id = Some(agent_id);
        let _ = self.ai_store.save_settings(&self.state.settings);
        self.state.drawer = AiDrawerTab::Chat;
        cx.notify();
    }

    pub(super) fn select_acp_agent(&mut self, agent_id: Option<String>, cx: &mut Context<Self>) {
        if let Some(id) = &agent_id
            && !self.state.settings.acp_agents.iter().any(|a| &a.id == id)
            && let Some(mut preset) = default_acp_presets().into_iter().find(|p| &p.id == id)
        {
            preset.enabled = true;
            self.state.settings.acp_agents.push(preset);
        }
        self.state.active_acp_agent_id = agent_id.clone();
        self.state.settings.active_acp_agent_id = agent_id.clone();
        self.state.active_popup = AiActivePopup::None;
        let _ = self.ai_store.save_settings(&self.state.settings);
        if let Some(id) = agent_id {
            let _ = self
                .command_tx
                .try_send(transom_core::AppCommand::Ai(Box::new(
                    transom_core::AiCommand::ProbeAcpModels { agent_id: id },
                )));
        }
        cx.notify();
    }
}
