//! 模型配置抽屉中的模型表格与行内请求模型 / 思考等级下拉渲染。

use std::rc::Rc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai_contracts::ThinkingEffort;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::app::ai::config_drawer::{
    ConfigDrawerActions, ConfigDrawerParams, DraftModelRow, render_config_drawer,
};
use crate::app::ai::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

impl AiAssistantView {
    /// 渲染带行内模型与思考等级下拉选择的模型配置表。
    pub(in crate::app::ai) fn render_models_table_section(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut table = v_flex().w_full().gap(rems(space::XXS));
        table = table.child(
            h_flex()
                .w_full()
                .gap(rems(space::XXS))
                .items_center()
                .child(
                    div().flex_1().child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(rust_i18n::t!("ai.cfg_col_display").to_string()),
                    ),
                )
                .child(
                    div().flex_1().child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(rust_i18n::t!("ai.cfg_col_request").to_string()),
                    ),
                )
                .child(
                    div().w(rems(5.2)).child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(rust_i18n::t!("ai.cfg_ctx_window_label").to_string()),
                    ),
                )
                .child(
                    div().w(rems(5.4)).child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(rust_i18n::t!("ai.cfg_col_effort").to_string()),
                    ),
                )
                .child(div().w(rems(1.8))),
        );

        let rm_tip = rust_i18n::t!("ai.cfg_remove_model").to_string();
        let unset_label = rust_i18n::t!("ai.cfg_effort_unset").to_string();
        let has_discovered = !self.state.config.discovered_models.is_empty();

        for (idx, row) in self.config_form.draft_models.iter().enumerate() {
            let effort_label = row
                .effort
                .map(|e| e.as_str().to_owned())
                .unwrap_or_else(|| unset_label.clone());
            let v_m_dd = cx.entity();
            let v_e_dd = cx.entity();
            let v_rm = cx.entity();

            let mut req_col = h_flex()
                .flex_1()
                .gap(px(2.0))
                .items_center()
                .child(div().flex_1().child(Input::new(&row.request_model)));
            if has_discovered {
                req_col = req_col.child(
                    Button::new(("ai-row-model-dd", idx))
                        .ghost()
                        .xsmall()
                        .accessibility_label(rust_i18n::t!("ai.picker_select_models").to_string())
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small())
                        .on_click(move |_, _, cx| {
                            v_m_dd.update(cx, |this, cx| {
                                this.model_picker.row_effort_dropdown = None;
                                this.model_picker.row_model_dropdown =
                                    if this.model_picker.row_model_dropdown == Some(idx) {
                                        None
                                    } else {
                                        Some(idx)
                                    };
                                cx.notify();
                            });
                        }),
                );
            }

            table = table.child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XXS))
                    .items_center()
                    .child(div().flex_1().child(Input::new(&row.display_name)))
                    .child(req_col)
                    .child(div().w(rems(5.2)).child(Input::new(&row.context_window)))
                    .child(
                        div().w(rems(5.4)).child(
                            Button::new(("ai-row-eff", idx))
                                .ghost()
                                .label(effort_label)
                                .on_click(move |_, _, cx| {
                                    v_e_dd.update(cx, |this, cx| {
                                        this.model_picker.row_model_dropdown = None;
                                        this.model_picker.row_effort_dropdown =
                                            if this.model_picker.row_effort_dropdown == Some(idx) {
                                                None
                                            } else {
                                                Some(idx)
                                            };
                                        cx.notify();
                                    });
                                }),
                        ),
                    )
                    .child(
                        div().w(rems(1.8)).child(
                            Button::new(("ai-row-rm", idx))
                                .ghost()
                                .accessibility_label(rm_tip.clone())
                                .tooltip(rm_tip.clone())
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                                .on_click(move |_, _, cx| {
                                    v_rm.update(cx, |this, cx| {
                                        this.remove_draft_model_row(idx, cx);
                                    });
                                }),
                        ),
                    ),
            );

            if self.model_picker.row_model_dropdown == Some(idx) {
                let mut dd = v_flex()
                    .w_full()
                    .max_h(px(160.0))
                    .overflow_y_scrollbar()
                    .p(rems(space::XXS))
                    .rounded(p.radius.sm)
                    .bg(p.surface.floating)
                    .border_1()
                    .border_color(p.border);
                for (m_idx, (mid, cw)) in self.state.config.discovered_models.iter().enumerate() {
                    let v_pick = cx.entity();
                    let m_id = mid.clone();
                    let m_cw = *cw;
                    dd = dd.child(
                        h_flex()
                            .id(("ai-row-mpick", idx * 1000 + m_idx))
                            .w_full()
                            .px(rems(space::XS))
                            .py(px(3.0))
                            .justify_between()
                            .rounded(p.radius.sm)
                            .cursor_pointer()
                            .hover(|s| s.bg(p.surface.hover))
                            .on_click(move |_, w, cx| {
                                let id = m_id.clone();
                                v_pick.update(cx, |this, cx| {
                                    this.set_row_request_model(idx, id, m_cw, w, cx);
                                });
                            })
                            .child(
                                div()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.foreground)
                                    .child(mid.clone()),
                            ),
                    );
                }
                table = table.child(dd);
            }

            if self.model_picker.row_effort_dropdown == Some(idx) {
                let mut opts: Vec<(Option<ThinkingEffort>, String)> =
                    vec![(None, unset_label.clone())];
                for eff in ThinkingEffort::ALL {
                    opts.push((Some(eff), eff.as_str().to_owned()));
                }
                let mut dd = h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(px(4.0))
                    .p(rems(space::XXS))
                    .rounded(p.radius.sm)
                    .bg(p.surface.floating)
                    .border_1()
                    .border_color(p.border);
                for (e_idx, (val, label)) in opts.into_iter().enumerate() {
                    let v_pick = cx.entity();
                    let active = row.effort == val;
                    let btn = Button::new(("ai-row-epick", idx * 20 + e_idx))
                        .xsmall()
                        .label(label);
                    let btn = if active { btn.primary() } else { btn.ghost() };
                    dd = dd.child(btn.on_click(move |_, _, cx| {
                        v_pick.update(cx, |this, cx| this.set_row_effort(idx, val, cx));
                    }));
                }
                table = table.child(dd);
            }
        }
        table.into_any_element()
    }

    pub(in crate::app::ai) fn add_empty_draft_model_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.config_form.draft_models.push(make_draft_model_row(
            "",
            "",
            Some(128_000),
            None,
            window,
            cx,
        ));
        cx.notify();
    }

    pub(in crate::app::ai) fn remove_draft_model_row(
        &mut self,
        idx: usize,
        cx: &mut Context<Self>,
    ) {
        if idx < self.config_form.draft_models.len() {
            self.config_form.draft_models.remove(idx);
            cx.notify();
        }
    }

    pub(in crate::app::ai) fn load_provider_into_form(
        &mut self,
        provider_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prov) = self
            .state
            .config
            .settings
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .cloned()
        else {
            return;
        };
        self.config_form.editing_provider = Some(prov.clone());
        self.state.config.selected_protocol = prov.protocol;
        self.config_form
            .provider_name_input
            .update(cx, |i, cx| i.set_value(prov.name, window, cx));
        self.config_form
            .base_url_input
            .update(cx, |i, cx| i.set_value(prov.base_url, window, cx));
        self.config_form
            .api_key_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.config_form.draft_models = prov
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
        if self.config_form.draft_models.is_empty() {
            self.config_form.draft_models.push(make_draft_model_row(
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

    pub(in crate::app::ai) fn render_config_body(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let models_section = self.render_models_table_section(p, cx);
        let picker_panel = self.render_model_picker_panel(p, cx);
        let policy_hooks_section =
            crate::app::ai::policy_hooks_drawer::render_policy_hooks_section(self, p, cx);
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
        let selected_acp_idx = self.config_form.selected_acp_idx;
        render_config_drawer(
            ConfigDrawerParams {
                active_section: self.state.view.config_section,
                settings: &self.state.config.settings,
                selected_protocol: self.state.config.selected_protocol,
                provider_name_input: &self.config_form.provider_name_input,
                api_key_input: &self.config_form.api_key_input,
                base_url_input: &self.config_form.base_url_input,
                models_section,
                picker_panel,
                has_discovered_models: !self.state.config.discovered_models.is_empty(),
                acp_cmd_input: &self.config_form.acp_cmd_input,
                selected_acp_idx,
                acp_install_status: self.state.config.acp_install_status.as_ref(),
                discovery_status: self.state.config.discovery_status.as_deref(),
                lang_select: &self.config_form.lang_select,
                running_behavior_select: &self.config_form.running_behavior_select,
                policy_hooks_section,
                p,
            },
            ConfigDrawerActions {
                on_select_section: Rc::new(move |section, _, cx| {
                    view_sec.update(cx, |this, cx| {
                        this.state.view.config_section = section;
                        cx.notify();
                    });
                }),
                on_select_protocol: Rc::new(move |proto, _, cx| {
                    view.update(cx, |this, cx| {
                        this.state.config.selected_protocol = proto;
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
}

pub(in crate::app::ai) fn make_draft_model_row(
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
