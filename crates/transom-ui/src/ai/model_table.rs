//! 模型配置抽屉中的模型表格与行内请求模型 / 思考等级下拉渲染。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::ThinkingEffort;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

impl AiAssistantView {
    /// 渲染带行内模型与思考等级下拉选择的模型配置表。
    pub(super) fn render_models_table_section(
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
        let has_discovered = !self.state.discovered_models.is_empty();

        for (idx, row) in self.draft_models.iter().enumerate() {
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
                for (m_idx, (mid, cw)) in self.state.discovered_models.iter().enumerate() {
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
}
