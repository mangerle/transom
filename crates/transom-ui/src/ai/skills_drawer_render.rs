//! AI 面板「技能（Skills）」抽屉视图渲染逻辑。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::SkillDefinition;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::config_drawer::{field_group, section_card};
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

impl AiAssistantView {
    pub(super) fn render_skills_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let v_import = cx.entity();
        let v_new = cx.entity();

        let mut root = v_flex()
            .flex_1()
            .w_full()
            .min_h_0()
            .p(rems(space::SM))
            .gap(rems(space::SM))
            .overflow_hidden();

        root = root.child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("ai.skills_title").to_string()),
                )
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .child(
                            Button::new("ai-skill-import")
                                .ghost()
                                .small()
                                .label(rust_i18n::t!("ai.skills_import").to_string())
                                .on_click(move |_, w, cx| {
                                    v_import.update(cx, |this, cx| {
                                        this.import_skill_via_dialog(w, cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("ai-skill-new")
                                .primary()
                                .small()
                                .label(rust_i18n::t!("ai.skills_new").to_string())
                                .on_click(move |_, w, cx| {
                                    v_new.update(cx, |this, cx| {
                                        this.open_skill_editor(None, w, cx);
                                    });
                                }),
                        ),
                ),
        );

        if let Some((ok, msg)) = &self.skills_drawer.status_notice {
            root = root.child(
                div()
                    .w_full()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .rounded(p.radius.sm)
                    .bg(p.secondary)
                    .border_1()
                    .border_color(if *ok { p.success } else { p.danger })
                    .text_size(rems(typography::CAPTION))
                    .text_color(if *ok { p.success } else { p.danger })
                    .child(msg.clone()),
            );
        }

        if self.skills_drawer.editing_id.is_some() {
            return root
                .child(self.render_skill_editor_form(p, cx))
                .into_any_element();
        }

        let mut list = v_flex()
            .flex_1()
            .w_full()
            .gap(rems(space::XS))
            .overflow_y_scrollbar();

        if self.skills_drawer.skills.is_empty() {
            list = list.child(
                div()
                    .p(rems(space::MD))
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(rust_i18n::t!("ai.skills_empty").to_string()),
            );
        } else {
            for (idx, skill) in self.skills_drawer.skills.iter().enumerate() {
                list = list.child(self.render_skill_row(idx, skill, p, cx));
            }
        }
        root.child(list).into_any_element()
    }

    fn render_skill_row(
        &self,
        idx: usize,
        skill: &SkillDefinition,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sid_toggle = skill.id.clone();
        let sid_del = skill.id.clone();
        let skill_edit = skill.clone();
        let v_toggle = cx.entity();
        let v_edit = cx.entity();
        let v_del = cx.entity();

        let source_badge = if skill.builtin {
            rust_i18n::t!("ai.skills_builtin_badge").to_string()
        } else {
            rust_i18n::t!("ai.skills_custom_badge").to_string()
        };
        let status_label = if skill.enabled {
            rust_i18n::t!("ai.skills_enabled").to_string()
        } else {
            rust_i18n::t!("ai.skills_disabled").to_string()
        };

        let mut actions = h_flex()
            .gap(rems(space::XXS))
            .items_center()
            .child(
                Button::new(("ai-skill-toggle", idx))
                    .ghost()
                    .small()
                    .label(status_label)
                    .on_click(move |_, _, cx| {
                        let id = sid_toggle.clone();
                        v_toggle.update(cx, |this, cx| this.toggle_skill_enabled(&id, cx));
                    }),
            )
            .child(
                Button::new(("ai-skill-edit", idx))
                    .ghost()
                    .small()
                    .label(rust_i18n::t!("ai.skills_edit").to_string())
                    .on_click(move |_, w, cx| {
                        v_edit.update(cx, |this, cx| {
                            this.open_skill_editor(Some(&skill_edit), w, cx);
                        });
                    }),
            );
        if !skill.builtin {
            actions = actions.child(
                Button::new(("ai-skill-del", idx))
                    .danger()
                    .small()
                    .label(rust_i18n::t!("ai.skills_delete").to_string())
                    .on_click(move |_, _, cx| {
                        let id = sid_del.clone();
                        v_del.update(cx, |this, cx| this.delete_skill_from_drawer(&id, cx));
                    }),
            );
        }

        let mut card = v_flex()
            .w_full()
            .p(rems(space::SM))
            .gap(px(4.0))
            .rounded(p.radius.md)
            .bg(p.surface.floating)
            .border_1()
            .border_color(p.divider(p.border))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap(rems(space::XS))
                    .child(
                        h_flex()
                            .flex_1()
                            .gap(px(6.0))
                            .items_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_color(if skill.enabled {
                                        p.primary
                                    } else {
                                        p.text.muted
                                    })
                                    .child(
                                        AppIconElement::new(AppIcon::Ui(UiIcon::LIGHTNING)).small(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(rems(typography::BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.foreground)
                                    .child(format!("{} ({})", skill.title, skill.command)),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(p.radius.sm)
                                    .bg(p.secondary)
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(source_badge),
                            ),
                    )
                    .child(actions),
            )
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.subtle)
                    .child(skill.description.clone()),
            );
        if !skill.resources.is_empty() {
            card = card.child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(
                        rust_i18n::t!("ai.skills_resources_count", count = skill.resources.len())
                            .to_string(),
                    ),
            );
        }
        card.into_any_element()
    }

    fn render_skill_editor_form(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let v_save = cx.entity();
        let v_cancel = cx.entity();
        let is_new = self
            .skills_drawer
            .editing_id
            .as_deref()
            .is_some_and(str::is_empty);
        let form_title = if is_new {
            rust_i18n::t!("ai.skill_form_new").to_string()
        } else {
            rust_i18n::t!("ai.skill_form_edit").to_string()
        };

        let card = section_card(form_title, None, p)
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::SM))
                    .items_start()
                    .child(div().flex_1().min_w_0().child(field_group(
                        rust_i18n::t!("ai.skill_form_id").to_string(),
                        Input::new(&self.skills_drawer.id_input).small(),
                        p,
                    )))
                    .child(div().flex_1().min_w_0().child(field_group(
                        rust_i18n::t!("ai.skill_form_title").to_string(),
                        Input::new(&self.skills_drawer.title_input).small(),
                        p,
                    ))),
            )
            .child(field_group(
                rust_i18n::t!("ai.skill_form_desc").to_string(),
                Input::new(&self.skills_drawer.desc_input).small(),
                p,
            ))
            .child(field_group(
                rust_i18n::t!("ai.skill_form_tags").to_string(),
                Input::new(&self.skills_drawer.tags_input).small(),
                p,
            ))
            .child(field_group(
                rust_i18n::t!("ai.skill_form_prompt").to_string(),
                div()
                    .w_full()
                    .p(rems(space::XS))
                    .rounded(p.radius.sm)
                    .bg(p.background)
                    .border_1()
                    .border_color(p.divider(p.border))
                    .child(
                        Textarea::new(&self.skills_drawer.body_input)
                            .appearance(false)
                            .bordered(false)
                            .h(rems(16.0)),
                    ),
                p,
            ))
            .child(
                h_flex()
                    .w_full()
                    .pt(px(2.0))
                    .justify_end()
                    .gap(rems(space::XS))
                    .child(
                        Button::new("ai-skill-cancel")
                            .ghost()
                            .small()
                            .label(rust_i18n::t!("ai.skills_cancel").to_string())
                            .on_click(move |_, _, cx| {
                                v_cancel.update(cx, |this, cx| {
                                    this.skills_drawer.editing_id = None;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("ai-skill-save")
                            .primary()
                            .small()
                            .label(rust_i18n::t!("ai.skills_save").to_string())
                            .on_click(move |_, _, cx| {
                                v_save.update(cx, |this, cx| this.save_skill_from_drawer(cx));
                            }),
                    ),
            );

        v_flex()
            .flex_1()
            .w_full()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(card)
            .into_any_element()
    }
}
