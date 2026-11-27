//! AI 面板「技能（Skills）」抽屉：浏览内置/自定义技能、开关启用、在线编辑与一键导入 `SKILL.md`。

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use transom_ai::{
    SkillDefinition, delete_custom_skill, import_skill_from_path, load_skills_with_disabled,
    save_custom_skill,
};
use transom_store::AiStore;

use super::view::AiAssistantView;

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
            &self.state.config.settings.disabled_skills,
        );
    }

    pub(super) fn toggle_skill_enabled(&mut self, skill_id: &str, cx: &mut Context<Self>) {
        let clean = skill_id.trim().to_ascii_lowercase();
        if let Some(pos) = self
            .state
            .config
            .settings
            .disabled_skills
            .iter()
            .position(|s| s.eq_ignore_ascii_case(&clean))
        {
            self.state.config.settings.disabled_skills.remove(pos);
        } else {
            self.state.config.settings.disabled_skills.push(clean);
        }
        self.refresh_skills_list();
        self.persist_settings_reported(cx);
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
}
