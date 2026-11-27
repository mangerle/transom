//! 模型配置抽屉中的“选择模型”多选面板与行内模型/思考等级下拉组件。

use std::collections::HashSet;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use transom_ai::ThinkingEffort;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::model_table::make_draft_model_row;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::search::contains_case_insensitive;
use crate::theme::tokens::{space, typography};

/// “选择模型”多选弹窗与行内下拉的状态。
pub(super) struct ModelPickerState {
    pub(super) open: bool,
    pub(super) search_input: Entity<InputState>,
    pub(super) selected_only: bool,
    pub(super) checked_ids: HashSet<String>,
    pub(super) row_model_dropdown: Option<usize>,
    pub(super) row_effort_dropdown: Option<usize>,
}

impl ModelPickerState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<AiAssistantView>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.picker_search_placeholder").to_string())
        });
        Self {
            open: false,
            search_input,
            selected_only: false,
            checked_ids: HashSet::new(),
            row_model_dropdown: None,
            row_effort_dropdown: None,
        }
    }
}

fn format_ctx_badge(ctx: u32) -> String {
    if ctx >= 1_000_000 {
        format!("{}M", ctx / 1_000_000)
    } else if ctx >= 1000 {
        format!("{}K", ctx / 1000)
    } else {
        ctx.to_string()
    }
}

impl AiAssistantView {
    /// 打开“选择模型”面板，并从当前草稿行初始化已勾选集合。
    pub(super) fn open_model_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker.checked_ids = self
            .draft_models
            .iter()
            .map(|r| r.request_model.read(cx).value().trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        self.model_picker.selected_only = false;
        self.model_picker.open = true;
        self.model_picker.row_model_dropdown = None;
        self.model_picker.row_effort_dropdown = None;
        cx.notify();
    }

    /// 当后台拉取到新模型列表时自动弹出“选择模型”面板。
    pub(super) fn sync_picker_from_discovered(&mut self, cx: &mut Context<Self>) {
        if self.state.discovered_models.is_empty() {
            return;
        }
        self.open_model_picker(cx);
    }

    /// 将“选择模型”勾选集合批量应用到草稿模型行（保留已有行自定义配置）。
    pub(super) fn apply_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let checked = &self.model_picker.checked_ids;
        self.draft_models.retain(|r| {
            let req = r.request_model.read(cx).value().trim().to_owned();
            !req.is_empty() && checked.contains(&req)
        });

        let existing: HashSet<String> = self
            .draft_models
            .iter()
            .map(|r| r.request_model.read(cx).value().trim().to_owned())
            .collect();

        for (id, ctx_win) in &self.state.discovered_models {
            if checked.contains(id) && !existing.contains(id) {
                self.draft_models.push(make_draft_model_row(
                    id,
                    id,
                    Some(ctx_win.unwrap_or(128_000)),
                    None,
                    window,
                    cx,
                ));
            }
        }
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
        self.model_picker.open = false;
        cx.notify();
    }

    pub(super) fn set_row_effort(
        &mut self,
        idx: usize,
        effort: Option<ThinkingEffort>,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.draft_models.get_mut(idx) {
            row.effort = effort;
        }
        self.model_picker.row_effort_dropdown = None;
        cx.notify();
    }

    pub(super) fn set_row_request_model(
        &mut self,
        idx: usize,
        model_id: String,
        ctx_win: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.draft_models.get(idx) {
            row.request_model
                .update(cx, |i, cx| i.set_value(model_id.clone(), window, cx));
            if row.display_name.read(cx).value().trim().is_empty() {
                row.display_name
                    .update(cx, |i, cx| i.set_value(model_id, window, cx));
            }
            if let Some(cw) = ctx_win {
                row.context_window
                    .update(cx, |i, cx| i.set_value(cw.to_string(), window, cx));
            }
        }
        self.model_picker.row_model_dropdown = None;
        cx.notify();
    }

    /// 渲染“选择模型”多选浮层卡片。
    pub(super) fn render_model_picker_panel(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.model_picker.open || self.state.discovered_models.is_empty() {
            return None;
        }
        let query = self
            .model_picker
            .search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let total = self.state.discovered_models.len();
        let selected_count = self.model_picker.checked_ids.len();

        let filtered: Vec<(String, Option<u32>)> = self
            .state
            .discovered_models
            .iter()
            .filter(|(id, _)| {
                let match_q = contains_case_insensitive(id, &query);
                let match_sel =
                    !self.model_picker.selected_only || self.model_picker.checked_ids.contains(id);
                match_q && match_sel
            })
            .cloned()
            .collect();

        let v_ref = cx.entity();
        let v_all = cx.entity();
        let v_none = cx.entity();
        let v_only = cx.entity();
        let v_cancel = cx.entity();
        let v_apply = cx.entity();
        let ref_tip = rust_i18n::t!("ai.picker_refresh").to_string();

        let mut list = v_flex()
            .w_full()
            .max_h(px(220.0))
            .overflow_y_scrollbar()
            .gap(px(2.0));
        for (idx, (id, ctx_win)) in filtered.iter().enumerate() {
            let checked = self.model_picker.checked_ids.contains(id);
            let mid = id.clone();
            let v_row = cx.entity();
            let mut row = h_flex()
                .id(("ai-picker-row", idx))
                .w_full()
                .px(rems(space::XS))
                .py(px(5.0))
                .gap(rems(space::XS))
                .items_center()
                .justify_between()
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, _, cx| {
                    let target = mid.clone();
                    v_row.update(cx, |this, cx| {
                        if !this.model_picker.checked_ids.remove(&target) {
                            this.model_picker.checked_ids.insert(target);
                        }
                        cx.notify();
                    });
                })
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(
                            div()
                                .w(px(15.0))
                                .h(px(15.0))
                                .flex_shrink_0()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(if checked { p.primary } else { p.border })
                                .bg(if checked {
                                    p.primary
                                } else {
                                    gpui_kit::transparent_black()
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(checked, |el| {
                                    el.child(
                                        AppIconElement::new(AppIcon::Ui(UiIcon::CHECK))
                                            .small()
                                            .monochrome(gpui_kit::white()),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .child(id.clone()),
                        ),
                );
            if let Some(cw) = ctx_win {
                row = row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(format_ctx_badge(*cw)),
                );
            }
            list = list.child(row);
        }

        Some(
            v_flex()
                .w_full()
                .p(rems(space::SM))
                .gap(rems(space::XS))
                .rounded(p.radius.md)
                .bg(p.surface.floating)
                .border_1()
                .border_color(p.border)
                .shadow_sm()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(
                            v_flex()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(rems(typography::BODY))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.foreground)
                                        .child(rust_i18n::t!("ai.picker_title").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(rems(typography::CAPTION))
                                        .text_color(p.text.muted)
                                        .child(rust_i18n::t!("ai.picker_subtitle").to_string()),
                                ),
                        )
                        .child(
                            Button::new("ai-picker-refresh")
                                .ghost()
                                .xsmall()
                                .accessibility_label(ref_tip.clone())
                                .tooltip(ref_tip)
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                                .on_click(move |_, _, cx| {
                                    v_ref.update(cx, |this, cx| this.discover_provider_models(cx));
                                }),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap(rems(space::XXS))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.model_picker.search_input)),
                        )
                        .child(
                            Button::new("ai-picker-all")
                                .ghost()
                                .xsmall()
                                .label(rust_i18n::t!("ai.picker_select_all").to_string())
                                .on_click(move |_, _, cx| {
                                    v_all.update(cx, |this, cx| {
                                        for (id, _) in &this.state.discovered_models {
                                            this.model_picker.checked_ids.insert(id.clone());
                                        }
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("ai-picker-none")
                                .ghost()
                                .xsmall()
                                .label(rust_i18n::t!("ai.picker_clear_all").to_string())
                                .on_click(move |_, _, cx| {
                                    v_none.update(cx, |this, cx| {
                                        this.model_picker.checked_ids.clear();
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(
                                    rust_i18n::t!(
                                        "ai.picker_selected_count",
                                        selected = selected_count,
                                        total = total
                                    )
                                    .to_string(),
                                ),
                        )
                        .child({
                            let btn = Button::new("ai-picker-only")
                                .xsmall()
                                .label(rust_i18n::t!("ai.picker_selected_only").to_string());
                            let btn = if self.model_picker.selected_only {
                                btn.primary()
                            } else {
                                btn.ghost()
                            };
                            btn.on_click(move |_, _, cx| {
                                v_only.update(cx, |this, cx| {
                                    this.model_picker.selected_only =
                                        !this.model_picker.selected_only;
                                    cx.notify();
                                });
                            })
                        }),
                )
                .child(list)
                .child(
                    h_flex()
                        .w_full()
                        .pt(px(4.0))
                        .justify_end()
                        .gap(rems(space::XS))
                        .child(
                            Button::new("ai-picker-cancel")
                                .ghost()
                                .small()
                                .label(rust_i18n::t!("common.cancel").to_string())
                                .on_click(move |_, _, cx| {
                                    v_cancel.update(cx, |this, cx| {
                                        this.model_picker.open = false;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("ai-picker-apply")
                                .primary()
                                .small()
                                .label(rust_i18n::t!("ai.picker_apply").to_string())
                                .on_click(move |_, w, cx| {
                                    v_apply.update(cx, |this, cx| this.apply_model_picker(w, cx));
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}
