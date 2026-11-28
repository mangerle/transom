//! AI 面板「技能（Skills）」独立子实体：浏览内置/自定义技能、开关启用、在线编辑与导入 `SKILL.md`。
//!
//! 设计原理（Why）：
//! - 将技能列表、5 个表单输入实体与后台导入/保存任务隔离到独立的 GPUI `Entity` 中，
//!   避免编辑技能表单或异步加载技能时触发主聊天面板全量重绘；
//! - 通过 [`SkillsDrawerEvent`] 向父视图冒泡“开关启用状态”和“技能列表更新”事件，
//!   严格遵循单向数据流。

use std::sync::Arc;

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use transom_ai_contracts::{
    SkillDefinition, delete_custom_skill, import_skill_from_path, load_skills_with_disabled,
    save_custom_skill,
};
use transom_store::AiStore;

use crate::app::spawn_blocking_local;
use crate::palette::Palette;

/// 技能抽屉向父视图冒泡的事件。
#[derive(Clone, Debug)]
pub(in crate::app::ai) enum SkillsDrawerEvent {
    /// 用户切换了指定技能的启用状态，需由父视图持久化 `disabled_skills`。
    ToggleSkillRequested(String),
    /// 技能列表加载或变更完成，同步最新可用技能快照供 Slash 菜单使用。
    SkillsLoaded(Vec<SkillDefinition>),
}

/// 技能管理独立子视图实体。
pub(in crate::app::ai) struct SkillsDrawerView {
    pub(in crate::app::ai) ai_store: Arc<AiStore>,
    pub(in crate::app::ai) disabled_skills: Vec<String>,
    pub(in crate::app::ai) skills: Vec<SkillDefinition>,
    pub(in crate::app::ai) editing_id: Option<String>,
    pub(in crate::app::ai) id_input: Entity<InputState>,
    pub(in crate::app::ai) title_input: Entity<InputState>,
    pub(in crate::app::ai) desc_input: Entity<InputState>,
    pub(in crate::app::ai) tags_input: Entity<InputState>,
    pub(in crate::app::ai) body_input: Entity<TextareaState>,
    pub(in crate::app::ai) status_notice: Option<(bool, String)>,
    pub(in crate::app::ai) load_seq: u64,
    pub(in crate::app::ai) _load_task: Option<Task<()>>,
    pub(in crate::app::ai) _mutation_task: Option<Task<()>>,
    pub(in crate::app::ai) _import_task: Option<Task<()>>,
}

impl EventEmitter<SkillsDrawerEvent> for SkillsDrawerView {}

impl SkillsDrawerView {
    pub(in crate::app::ai) fn new(
        ai_store: Arc<AiStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mk_input = |key: &str, window: &mut Window, cx: &mut Context<Self>| {
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
            ai_store,
            disabled_skills: Vec::new(),
            skills: Vec::new(),
            editing_id: None,
            id_input,
            title_input,
            desc_input,
            tags_input,
            body_input,
            status_notice: None,
            load_seq: 0,
            _load_task: None,
            _mutation_task: None,
            _import_task: None,
        }
    }

    pub(in crate::app::ai) fn set_disabled_and_refresh(
        &mut self,
        disabled_skills: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.disabled_skills = disabled_skills;
        self.refresh_skills_list(cx);
    }

    pub(in crate::app::ai) fn refresh_skills_list(&mut self, cx: &mut Context<Self>) {
        self.load_seq = self.load_seq.wrapping_add(1);
        let load_seq = self.load_seq;
        let dir = self.ai_store.custom_skills_dir();
        let disabled = self.disabled_skills.clone();
        self._load_task = Some(cx.spawn(async move |this, cx| {
            let loaded =
                spawn_blocking_local(move || load_skills_with_disabled(Some(&dir), &disabled))
                    .await;
            if this
                .update(cx, |this, cx| {
                    if this.load_seq != load_seq {
                        return;
                    }
                    if let Some(skills) = loaded {
                        this.skills = skills.clone();
                        cx.emit(SkillsDrawerEvent::SkillsLoaded(skills));
                    } else {
                        this.status_notice = Some((
                            false,
                            rust_i18n::t!("ai.background_task_failed").to_string(),
                        ));
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("AI 技能列表读取完成时视图已关闭，结果被丢弃");
            }
        }));
    }

    pub(in crate::app::ai) fn toggle_skill_enabled(
        &mut self,
        skill_id: &str,
        cx: &mut Context<Self>,
    ) {
        let clean = skill_id.trim().to_ascii_lowercase();
        if let Some(pos) = self
            .disabled_skills
            .iter()
            .position(|s| s.eq_ignore_ascii_case(&clean))
        {
            self.disabled_skills.remove(pos);
        } else {
            self.disabled_skills.push(clean.clone());
        }
        self.refresh_skills_list(cx);
        cx.emit(SkillsDrawerEvent::ToggleSkillRequested(clean));
    }

    pub(in crate::app::ai) fn open_skill_editor(
        &mut self,
        skill: Option<&SkillDefinition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_notice = None;
        match skill {
            Some(s) => {
                self.editing_id = Some(s.id.clone());
                let pairs = [
                    (&self.id_input, s.id.clone()),
                    (&self.title_input, s.title.clone()),
                    (&self.desc_input, s.description.clone()),
                    (&self.tags_input, s.tags.join(", ")),
                ];
                for (input, val) in pairs {
                    input.update(cx, |i, cx| i.set_value(val, window, cx));
                }
                let body = s.prompt_template.clone();
                self.body_input
                    .update(cx, |i, cx| i.set_value(body, window, cx));
            }
            None => {
                self.editing_id = Some(String::new());
                for input in [
                    &self.id_input,
                    &self.title_input,
                    &self.desc_input,
                    &self.tags_input,
                ] {
                    input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                }
                self.body_input
                    .update(cx, |i, cx| i.set_value(String::new(), window, cx));
            }
        }
        cx.notify();
    }

    pub(in crate::app::ai) fn save_skill_from_drawer(&mut self, cx: &mut Context<Self>) {
        let id = self.id_input.read(cx).value().to_string();
        let title = self.title_input.read(cx).value().to_string();
        let desc = self.desc_input.read(cx).value().to_string();
        let tags_raw = self.tags_input.read(cx).value().to_string();
        let body = self.body_input.read(cx).value().to_string();
        let tags: Vec<String> = tags_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();

        let dir = self.ai_store.custom_skills_dir();
        self._mutation_task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_blocking_local(move || {
                save_custom_skill(&dir, &id, &title, &desc, &tags, &body)
            })
            .await;
            if this
                .update(cx, |this, cx| {
                    match result {
                        Some(Ok(_)) => {
                            this.editing_id = None;
                            this.status_notice =
                                Some((true, rust_i18n::t!("ai.skills_saved_ok").to_string()));
                            this.refresh_skills_list(cx);
                        }
                        Some(Err(err)) => {
                            this.status_notice = Some((false, err.to_string()));
                        }
                        None => {
                            this.status_notice = Some((
                                false,
                                rust_i18n::t!("ai.background_task_failed").to_string(),
                            ));
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("AI 技能保存完成时视图已关闭，结果被丢弃");
            }
        }));
    }

    pub(in crate::app::ai) fn delete_skill_from_drawer(
        &mut self,
        skill_id: &str,
        cx: &mut Context<Self>,
    ) {
        let dir = self.ai_store.custom_skills_dir();
        let skill_id = skill_id.to_owned();
        self._mutation_task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_blocking_local(move || delete_custom_skill(&dir, &skill_id)).await;
            if this
                .update(cx, |this, cx| {
                    match result {
                        Some(Ok(true)) => this.refresh_skills_list(cx),
                        Some(Ok(false)) => {
                            this.status_notice = Some((
                                false,
                                rust_i18n::t!("ai.skills_delete_missing").to_string(),
                            ));
                        }
                        Some(Err(err)) => {
                            this.status_notice = Some((false, err.to_string()));
                        }
                        None => {
                            this.status_notice = Some((
                                false,
                                rust_i18n::t!("ai.background_task_failed").to_string(),
                            ));
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("AI 技能删除完成时视图已关闭，结果被丢弃");
            }
        }));
    }

    pub(in crate::app::ai) fn import_skill_via_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: None,
        });
        let dir = self.ai_store.custom_skills_dir();
        self._import_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(src) = paths.into_iter().next() else {
                return;
            };
            let result = spawn_blocking_local(move || import_skill_from_path(&src, &dir)).await;
            if this
                .update_in(cx, |this, _window, cx| {
                    match result {
                        Some(Ok(skill)) => {
                            this.status_notice = Some((
                                true,
                                rust_i18n::t!("ai.skills_imported_ok", name = skill.title)
                                    .to_string(),
                            ));
                            this.refresh_skills_list(cx);
                        }
                        Some(Err(err)) => {
                            this.status_notice = Some((false, err.to_string()));
                        }
                        None => {
                            this.status_notice = Some((
                                false,
                                rust_i18n::t!("ai.background_task_failed").to_string(),
                            ));
                        }
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("AI 技能导入完成时视图已关闭，结果被丢弃");
            }
        }));
    }
}

impl Render for SkillsDrawerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_skills_body(&p, cx)
    }
}
