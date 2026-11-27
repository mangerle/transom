//! 二选一 / 多选项交互式确认选择卡片：支持点击切换单选或多选、自定义输入、跳过与提交。

use std::collections::HashSet;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use transom_ai::{ExtractedChoices, extract_prompt_choices};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::state::UiChatItem;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

type ChoiceIndexCb = Box<dyn Fn(usize, &mut Window, &mut App)>;
type ChoiceActionCb = Box<dyn Fn(&mut Window, &mut App)>;

/// 收敛交互式选择卡片的局部 UI 状态，避免零散字段堆叠在主视图中。
pub(super) struct ChoiceCardState {
    pub(super) custom_input: Entity<InputState>,
    pub(super) selected_idx: usize,
    pub(super) multi_selected: HashSet<usize>,
    pub(super) collapsed: bool,
    pub(super) dismissed_id: Option<String>,
}

impl ChoiceCardState {
    pub(super) fn new(custom_input: Entity<InputState>) -> Self {
        Self {
            custom_input,
            selected_idx: 0,
            multi_selected: HashSet::new(),
            collapsed: false,
            dismissed_id: None,
        }
    }

    pub(super) fn dismiss_and_reset(&mut self, item_id: String) {
        self.dismissed_id = Some(item_id);
        self.selected_idx = 0;
        self.multi_selected.clear();
        self.collapsed = false;
    }
}

/// 交互式确认卡片渲染参数与回调。
pub(super) struct ChoiceCardCallbacks {
    pub(super) on_select_option: ChoiceIndexCb,
    pub(super) on_toggle_collapse: ChoiceActionCb,
    pub(super) on_skip: ChoiceActionCb,
    pub(super) on_submit: ChoiceActionCb,
}

fn number_badge(num: usize, active: bool, p: &Palette) -> AnyElement {
    div()
        .w(px(20.0))
        .h(px(20.0))
        .rounded(p.radius.sm)
        .bg(if active { p.background } else { p.secondary })
        .border_1()
        .border_color(p.divider(p.border))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .text_size(rems(typography::CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if active { p.foreground } else { p.text.muted })
        .child(format!("{num}"))
        .into_any_element()
}

/// 渲染参考样式的圆角交互式选择卡片（支持单选与多选模式）。
pub(super) fn render_choice_card(
    choices: &ExtractedChoices,
    selected_idx: usize,
    multi_selected: &HashSet<usize>,
    collapsed: bool,
    custom_input: &Entity<InputState>,
    p: &Palette,
    cb: ChoiceCardCallbacks,
) -> AnyElement {
    let dismiss_tip = rust_i18n::t!("ai.choice_dismiss").to_string();
    let toggle_tip = rust_i18n::t!("ai.choice_toggle").to_string();
    let skip_rc = std::rc::Rc::new(cb.on_skip);
    let skip_top = skip_rc.clone();
    let title = if choices.prompt.is_empty() {
        rust_i18n::t!("ai.choice_card_title").to_string()
    } else {
        choices.prompt.clone()
    };
    let chevron = if collapsed {
        UiIcon::CHEVRON_UP
    } else {
        UiIcon::CHEVRON_DOWN
    };

    let mut card = v_flex()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(px(12.0))
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .shadow_md()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .gap(rems(space::XS))
                .child(
                    h_flex()
                        .flex_1()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(
                            div()
                                .text_color(p.text.muted)
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::HELP)).small()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(rems(typography::BODY))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .child(title),
                        ),
                )
                .child(
                    h_flex()
                        .gap(px(2.0))
                        .items_center()
                        .child(
                            Button::new("ai-choice-toggle")
                                .ghost()
                                .accessibility_label(toggle_tip.clone())
                                .tooltip(toggle_tip)
                                .child(AppIconElement::new(AppIcon::Ui(chevron)).small())
                                .on_click(move |_, w, cx| (cb.on_toggle_collapse)(w, cx)),
                        )
                        .child(
                            Button::new("ai-choice-dismiss")
                                .ghost()
                                .accessibility_label(dismiss_tip.clone())
                                .tooltip(dismiss_tip)
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                                .on_click(move |_, w, cx| skip_top(w, cx)),
                        ),
                ),
        );

    if collapsed {
        return card.into_any_element();
    }

    let select_rc = std::rc::Rc::new(cb.on_select_option);
    let active_idx = selected_idx.min(choices.options.len().saturating_sub(1));
    for (idx, option) in choices.options.iter().enumerate() {
        let is_selected = if choices.multi_select {
            multi_selected.contains(&idx)
        } else {
            idx == active_idx
        };
        let select = select_rc.clone();
        let mut row = h_flex()
            .id(("ai-choice-opt", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .items_start()
            .rounded(p.radius.md)
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| select(idx, w, cx))
            .child(number_badge(idx + 1, is_selected, p))
            .child(
                div()
                    .flex_1()
                    .text_size(rems(typography::BODY))
                    .text_color(p.foreground)
                    .child(option.clone()),
            );
        if choices.multi_select && is_selected {
            row = row.child(
                div()
                    .text_color(p.success)
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
            );
        }
        if is_selected {
            row = row.bg(p.surface.selected);
        }
        card = card.child(row);
    }

    let other_num = choices.options.len() + 1;
    card.child(
        h_flex()
            .w_full()
            .pt(px(2.0))
            .gap(rems(space::XS))
            .items_center()
            .child(
                h_flex()
                    .flex_1()
                    .px(rems(space::XS))
                    .py(px(2.0))
                    .gap(rems(space::XS))
                    .items_center()
                    .rounded(p.radius.md)
                    .bg(p.secondary)
                    .child(number_badge(other_num, false, p))
                    .child(
                        div().flex_1().child(
                            Input::new(custom_input)
                                .appearance(false)
                                .bordered(false)
                                .small(),
                        ),
                    ),
            )
            .child(
                Button::new("ai-choice-skip")
                    .ghost()
                    .label(rust_i18n::t!("ai.choice_skip").to_string())
                    .on_click(move |_, w, cx| skip_rc(w, cx)),
            )
            .child(
                Button::new("ai-choice-submit")
                    .primary()
                    .label(rust_i18n::t!("ai.choice_submit").to_string())
                    .on_click(move |_, w, cx| (cb.on_submit)(w, cx)),
            ),
    )
    .into_any_element()
}

/// 创建自定义选项输入框并订阅回车提交事件。
pub(super) fn init_choice_input(
    window: &mut Window,
    cx: &mut Context<AiAssistantView>,
) -> (Entity<InputState>, Subscription) {
    let input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(rust_i18n::t!("ai.choice_custom_placeholder").to_string())
    });
    let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. })
            && let Some(UiChatItem::AssistantText { id, .. }) = this.state.items.last()
        {
            let id = id.clone();
            this.submit_custom_choice(&id, window, cx);
        }
    });
    (input, sub)
}

impl AiAssistantView {
    /// 当最后一条已完成回复包含交互式选择列表时渲染确认卡片。
    pub(super) fn maybe_render_choice_card(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.state.running_run_id.is_some() || !self.state.pending_approvals.is_empty() {
            return None;
        }
        let UiChatItem::AssistantText {
            id,
            text,
            streaming: false,
            ..
        } = self.state.items.last()?
        else {
            return None;
        };
        if self.choice_card.dismissed_id.as_deref() == Some(id.as_str()) {
            return None;
        }
        let choices = extract_prompt_choices(text)?;
        let item_id = id.clone();
        let v_sel = cx.entity();
        let v_tog = cx.entity();
        let v_skip = cx.entity();
        let v_sub = cx.entity();
        let opts_for_submit = choices.options.clone();
        let id_skip = item_id.clone();
        let id_sub = item_id;
        let is_multi = choices.multi_select;
        Some(render_choice_card(
            &choices,
            self.choice_card.selected_idx,
            &self.choice_card.multi_selected,
            self.choice_card.collapsed,
            &self.choice_card.custom_input,
            p,
            ChoiceCardCallbacks {
                on_select_option: Box::new(move |idx, _, cx| {
                    v_sel.update(cx, |this, cx| {
                        if is_multi {
                            if !this.choice_card.multi_selected.remove(&idx) {
                                this.choice_card.multi_selected.insert(idx);
                            }
                        } else {
                            this.choice_card.selected_idx = idx;
                        }
                        cx.notify();
                    });
                }),
                on_toggle_collapse: Box::new(move |_, cx| {
                    v_tog.update(cx, |this, cx| {
                        this.choice_card.collapsed = !this.choice_card.collapsed;
                        cx.notify();
                    });
                }),
                on_skip: Box::new(move |_, cx| {
                    let id_done = id_skip.clone();
                    v_skip.update(cx, |this, cx| {
                        this.choice_card.dismiss_and_reset(id_done);
                        cx.notify();
                    });
                }),
                on_submit: Box::new(move |w, cx| {
                    let id_done = id_sub.clone();
                    let opts = opts_for_submit.clone();
                    v_sub.update(cx, |this, cx| {
                        this.submit_choice_answer(&id_done, is_multi, &opts, w, cx);
                    });
                }),
            },
        ))
    }

    pub(super) fn submit_choice_answer(
        &mut self,
        item_id: &str,
        is_multi: bool,
        options: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let custom = self
            .choice_card
            .custom_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let mut selected_parts: Vec<String> = Vec::new();
        if is_multi {
            for (idx, opt) in options.iter().enumerate() {
                if self.choice_card.multi_selected.contains(&idx) {
                    selected_parts.push(format!("{}. {opt}", idx + 1));
                }
            }
            if !custom.is_empty() {
                selected_parts.push(custom);
            }
        } else if !custom.is_empty() {
            selected_parts.push(custom);
        } else if let Some(opt) = options.get(
            self.choice_card
                .selected_idx
                .min(options.len().saturating_sub(1)),
        ) {
            selected_parts.push(opt.clone());
        }
        if selected_parts.is_empty() {
            return;
        }
        let answer = selected_parts.join("\n");
        self.choice_card.dismiss_and_reset(item_id.to_owned());
        self.choice_card
            .custom_input
            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.send_prompt_text(answer, cx);
    }

    pub(super) fn submit_custom_choice(
        &mut self,
        item_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let val = self
            .choice_card
            .custom_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if val.is_empty() {
            return;
        }
        self.choice_card.dismiss_and_reset(item_id.to_owned());
        self.choice_card
            .custom_input
            .update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.send_prompt_text(val, cx);
    }
}
