//! AI 面板「技能（Skills）」抽屉：浏览内置/自定义技能、开关启用、在线编辑与一键导入 `SKILL.md`。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::{
    SkillDefinition, delete_custom_skill, import_skill_from_path, load_skills_with_disabled,
    save_custom_skill,
};
use transom_store::AiStore;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::config_drawer::{field_group, section_card};
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 技能管理抽屉内部状态。
pub(super) struct SkillsDrawerState {
    pub(super) skills: Vec<SkillDefinition>,
    pub(super) editing_id: Option<String>,
    pub(super) id_input: Entity<InputState>,
    pub(super) title_input: Entity<InputState>,
    pub(super) desc_input: Entity<InputState>,
    pub(super) tags_input: Entity<InputState>,
    pub(super) body_input: Entity<TextareaState>,
    pub(super) status_notice: Option<(bool, String)>,
    pub(super) _import_task: Option<Task<()>>,
}

impl SkillsDrawerState {
    pub(super) fn new(
        ai_store: &AiStore,
        disabled_skills: &[String],
        window: &mut Window,
        cx: &mut Context<AiAssistantView>,
    ) -> Self {
        let skills =
            load_skills_with_disabled(Some(&ai_store.custom_skills_dir()), disabled_skills);
        let mk_input = |key: &str, window: &mut Window, cx: &mut Context<AiAssistantView>| {
            let ph = rust_i18n::t!(key).to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(ph))
        };
        let id_input = mk_input("ai.skills_id_placeholder", window, cx);
        let title_input = mk_input("ai.skills_title_placeholder", window, cx);
        let desc_input = mk_input("ai.skills_desc_placeholder", window, cx);
        let tags_input = mk_input("ai.skills_tags_placeholder", window, cx);
        let body_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.skills_body_placeholder").to_string())
        });
        Self {
            skills,
            editing_id: None,
            id_input,
            title_input,
            desc_input,
            tags_input,
            body_input,
            status_notice: None,
            _import_task: None,
        }
    }
}

impl AiAssistantView {
    pub(super) fn refresh_skills_list(&mut self) {
        self.skills_drawer.skills = load_skills_with_disabled(
            Some(&self.ai_store.custom_skills_dir()),
            &self.state.settings.disabled_skills,
        );
    }

    pub(super) fn toggle_skill_enabled(&mut self, skill_id: &str, cx: &mut Context<Self>) {
        let clean = skill_id.trim().to_ascii_lowercase();
        if let Some(pos) = self
            .state
            .settings
            .disabled_skills
            .iter()
            .position(|s| s.eq_ignore_ascii_case(&clean))
        {
            self.state.settings.disabled_skills.remove(pos);
        } else {
            self.state.settings.disabled_skills.push(clean);
        }
        let _ = self.ai_store.save_settings(&self.state.settings);
        self.refresh_skills_list();
        cx.notify();
    }

    pub(super) fn open_skill_editor(
        &mut self,
        skill: Option<&SkillDefinition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.skills_drawer.status_notice = None;
        match skill {
            Some(s) => {
                self.skills_drawer.editing_id = Some(s.id.clone());
                let pairs = [
                    (&self.skills_drawer.id_input, s.id.clone()),
                    (&self.skills_drawer.title_input, s.title.clone()),
                    (&self.skills_drawer.desc_input, s.description.clone()),
                    (&self.skills_drawer.tags_input, s.tags.join(", ")),
                ];
                for (input, val) in pairs {
                    input.update(cx, |i, cx| i.set_value(val, window, cx));
                }
                let body = s.prompt_template.clone();
                self.skills_drawer
                    .body_input
                    .update(cx, |i, cx| i.set_value(body, window, cx));
            }
            None => {
                self.skills_drawer.editing_id = Some(String::new());
                for input in [
                    &self.skills_drawer.id_input,
                    &self.skills_drawer.title_input,
                    &self.skills_drawer.desc_input,
                    &self.skills_drawer.tags_input,
                ] {
                    input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                }
                self.skills_drawer
                    .body_input
                    .update(cx, |i, cx| i.set_value(String::new(), window, cx));
            }
        }
        cx.notify();
    }

    pub(super) fn save_skill_from_drawer(&mut self, cx: &mut Context<Self>) {
        let id = self.skills_drawer.id_input.read(cx).value().to_string();
        let title = self.skills_drawer.title_input.read(cx).value().to_string();
        let desc = self.skills_drawer.desc_input.read(cx).value().to_string();
        let tags_raw = self.skills_drawer.tags_input.read(cx).value().to_string();
        let body = self.skills_drawer.body_input.read(cx).value().to_string();
        let tags: Vec<String> = tags_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();

        match save_custom_skill(
            &self.ai_store.custom_skills_dir(),
            &id,
            &title,
            &desc,
            &tags,
            &body,
        ) {
            Ok(_) => {
                self.skills_drawer.editing_id = None;
                self.skills_drawer.status_notice =
                    Some((true, rust_i18n::t!("ai.skills_saved_ok").to_string()));
                self.refresh_skills_list();
            }
            Err(err) => {
                self.skills_drawer.status_notice = Some((false, err.to_string()));
            }
        }
        cx.notify();
    }

    pub(super) fn delete_skill_from_drawer(&mut self, skill_id: &str, cx: &mut Context<Self>) {
        match delete_custom_skill(&self.ai_store.custom_skills_dir(), skill_id) {
            Ok(_) => {
                self.refresh_skills_list();
            }
            Err(err) => {
                self.skills_drawer.status_notice = Some((false, err.to_string()));
            }
        }
        cx.notify();
    }

    pub(super) fn import_skill_via_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: None,
        });
        let dir = self.ai_store.custom_skills_dir();
        let bg = cx.background_executor().clone();
        self.skills_drawer._import_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(src) = paths.into_iter().next() else {
                return;
            };
            let res = bg
                .spawn(async move { import_skill_from_path(&src, &dir) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(skill) => {
                        this.skills_drawer.status_notice = Some((
                            true,
                            rust_i18n::t!("ai.skills_imported_ok", name = skill.title).to_string(),
                        ));
                        this.refresh_skills_list();
                    }
                    Err(err) => {
                        this.skills_drawer.status_notice = Some((false, err.to_string()));
                    }
                }
                cx.notify();
            });
        }));
    }

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
            "新建技能".to_owned()
        } else {
            "编辑技能".to_owned()
        };

        let card = section_card(form_title, None, p)
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::SM))
                    .items_start()
                    .child(
                        div().flex_1().min_w_0().child(field_group(
                            "技能 ID（斜杠命令）".to_owned(),
                            Input::new(&self.skills_drawer.id_input).small(),
                            p,
                        )),
                    )
                    .child(
                        div().flex_1().min_w_0().child(field_group(
                            "技能名称".to_owned(),
                            Input::new(&self.skills_drawer.title_input).small(),
                            p,
                        )),
                    ),
            )
            .child(field_group(
                "简短描述".to_owned(),
                Input::new(&self.skills_drawer.desc_input).small(),
                p,
            ))
            .child(field_group(
                "标签（逗号分隔，可选）".to_owned(),
                Input::new(&self.skills_drawer.tags_input).small(),
                p,
            ))
            .child(field_group(
                "提示词模板（Prompt）".to_owned(),
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
