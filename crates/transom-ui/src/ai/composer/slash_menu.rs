//! AI 面板斜杠命令补全弹层与空状态引导页。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InlineToken;
use gpui_kit::*;
use transom_ai_contracts::SkillDefinition;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use crate::app::ai::state::AiDrawerTab;
use crate::app::ai::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 斜杠命令补全弹层中的单条候选指令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app::ai) struct SlashItem {
    pub(in crate::app::ai) command: String,
    pub(in crate::app::ai) description: String,
    pub(in crate::app::ai) builtin: bool,
}

/// 提取输入文本末尾正在输入的 `/query`（返回 `/` 起始字节偏移与查询词）。
///
/// 仅当 `/` 位于开头或前一个字符为空白、且之后不含空白或嵌套 `/`（避免误触 `/var/log`）时触发。
pub(in crate::app::ai) fn extract_active_slash(input: &str) -> Option<(usize, &str)> {
    let slash_pos = input.rfind('/')?;
    if slash_pos > 0 {
        let prev = input[..slash_pos].chars().next_back()?;
        if !prev.is_whitespace() {
            return None;
        }
    }
    let query = &input[slash_pos..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    Some((slash_pos, query))
}

/// 根据当前输入框内容筛选斜杠命令；支持在 `@主机` 后继续输入 `/命令`。
pub(in crate::app::ai) fn filter_slash_items(
    input_text: &str,
    skills: &[SkillDefinition],
) -> Vec<SlashItem> {
    let Some((_, active_query)) = extract_active_slash(input_text) else {
        return Vec::new();
    };
    let query = active_query.to_lowercase();
    let mut all = vec![
        SlashItem {
            command: "/clear".to_owned(),
            description: rust_i18n::t!("ai.slash_clear_desc").to_string(),
            builtin: true,
        },
        SlashItem {
            command: "/compact".to_owned(),
            description: rust_i18n::t!("ai.slash_compact_desc").to_string(),
            builtin: true,
        },
    ];
    for skill in skills.iter().filter(|s| s.enabled) {
        let cmd = if skill.command.starts_with('/') {
            skill.command.clone()
        } else {
            format!("/{}", skill.command)
        };
        if !all.iter().any(|it| it.command.eq_ignore_ascii_case(&cmd)) {
            let desc = if skill.title.trim().is_empty() || skill.title == skill.id {
                skill.description.clone()
            } else {
                format!("{} · {}", skill.title, skill.description)
            };
            all.push(SlashItem {
                command: cmd,
                description: desc,
                builtin: skill.builtin,
            });
        }
    }
    all.into_iter()
        .filter(|it| it.command.to_lowercase().starts_with(&query))
        .collect()
}

/// 计算斜杠命令列表上下移动后的索引（支持首尾回绕）。
pub(in crate::app::ai) fn step_slash_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let cur = current.min(len - 1) as isize;
    let total = len as isize;
    (cur + delta).rem_euclid(total) as usize
}

fn slash_item_icon(command: &str) -> AppIcon {
    match command {
        "/clear" => AppIcon::Ui(UiIcon::RESET),
        "/compact" => AppIcon::Ui(UiIcon::COLLAPSE_ALL),
        "/disk" => AppIcon::Custom(CustomIcon::DISK),
        "/docker" => AppIcon::Custom(CustomIcon::DOCKER),
        _ => AppIcon::Ui(UiIcon::LIGHTNING),
    }
}

/// 渲染输入框正上方的斜杠命令补全弹层。
pub(in crate::app::ai) fn render_slash_popover(
    items: &[SlashItem],
    selected_idx: usize,
    scroll_handle: &ScrollHandle,
    p: &Palette,
    on_pick: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let active_idx = selected_idx.min(items.len().saturating_sub(1));
    let mut list = v_flex()
        .id("ai-slash-list")
        .w_full()
        .max_h(px(240.0))
        .overflow_y_scroll()
        .track_scroll(scroll_handle);
    for (idx, item) in items.iter().enumerate() {
        let is_selected = idx == active_idx;
        let cmd = item.command.clone();
        let pick = on_pick.clone();
        let mut row = h_flex()
            .id(("ai-slash-item", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::SM))
            .justify_between()
            .items_center()
            .rounded(p.radius.sm)
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| pick(cmd.clone(), w, cx))
            .child(
                h_flex()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(
                        div()
                            .text_color(if is_selected { p.primary } else { p.text.muted })
                            .child(AppIconElement::new(slash_item_icon(&item.command)).small()),
                    )
                    .child(
                        div()
                            .text_size(rems(typography::BODY))
                            .font_weight(if is_selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(p.foreground)
                            .child(item.command.clone()),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .gap(rems(space::XS))
                    .items_center()
                    .justify_end()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_right()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(item.description.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(p.radius.sm)
                            .bg(p.secondary)
                            .text_size(rems(typography::CAPTION))
                            .text_color(if item.builtin {
                                p.text.muted
                            } else {
                                p.primary
                            })
                            .child(if item.builtin {
                                rust_i18n::t!("ai.skills_builtin_badge").to_string()
                            } else {
                                rust_i18n::t!("ai.skills_custom_badge").to_string()
                            }),
                    ),
            );
        if is_selected {
            row = row.bg(p.surface.selected);
        }
        list = list.child(row);
    }
    v_flex()
        .w_full()
        .p(rems(space::XXS))
        .gap(px(2.0))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .shadow_md()
        .child(list)
        .into_any_element()
}

impl AiAssistantView {
    /// 获取当前输入框匹配的斜杠命令候选项（跳过已转换为行内胶囊的片段）。
    /// 判断当前输入是否处于按 Esc 手动收起补全的状态。
    pub(in crate::app::ai) fn is_completion_dismissed(&self, cx: &App) -> bool {
        let Some(snapshot) = &self.completion_dismissed_input else {
            return false;
        };
        self.composer_input.read(cx).value() == snapshot.as_str()
    }

    pub(in crate::app::ai) fn current_slash_items(&self, cx: &App) -> Vec<SlashItem> {
        if self.is_completion_dismissed(cx) {
            return Vec::new();
        }
        let input = self.composer_input.read(cx);
        let current_input = input.value().to_string();
        if let Some((pos, _)) = extract_active_slash(&current_input)
            && input
                .tokens()
                .iter()
                .any(|span| span.range().start <= pos && pos < span.range().end)
        {
            return Vec::new();
        }
        filter_slash_items(&current_input, &self.available_skills)
    }

    /// 将选中的斜杠命令替换为行内胶囊芯片（开头的 `/clear` 且无前置内容时直接清空会话）。
    pub(in crate::app::ai) fn apply_slash_command(
        &mut self,
        cmd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.slash_selected_idx = 0;
        let current = self.composer_input.read(cx).value().to_string();
        let slash_pos = extract_active_slash(&current)
            .map(|(pos, _)| pos)
            .unwrap_or(0);
        if cmd == "/clear" && slash_pos == 0 && current[..slash_pos].trim().is_empty() {
            self.composer_input
                .update(cx, |i, cx| i.set_value(String::new(), window, cx));
            self.last_composer_len = 0;
            self.clear_conversation(cx);
        } else {
            let range = slash_pos..current.len();
            let token = InlineToken::new(format!("slash:{cmd}"), cmd);
            self.composer_input.update(cx, |i, cx| {
                if i.replace_range_with_token(range, token, window, cx).is_ok() {
                    i.insert(" ", window, cx);
                }
            });
            self.last_composer_len = self.composer_input.read(cx).value().len();
            cx.notify();
        }
    }

    /// 若斜杠补全弹层当前可见，则应用当前高亮命令并返回 `true`。
    pub(in crate::app::ai) fn try_apply_selected_slash(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let items = self.current_slash_items(cx);
        let Some(item) = items.get(self.slash_selected_idx.min(items.len().saturating_sub(1)))
        else {
            return false;
        };
        let cmd = item.command.clone();
        self.apply_slash_command(&cmd, window, cx);
        true
    }
}

/// 注册斜杠补全全局按键拦截器，在输入框 action 派发前拦截上下方向键与 Tab / Enter。
pub(in crate::app::ai) fn register_slash_key_interceptor(
    cx: &mut Context<AiAssistantView>,
) -> Subscription {
    let weak_keys = cx.weak_entity();
    cx.intercept_keystrokes(move |event, window, cx| {
        let m = &event.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function || m.shift {
            return;
        }
        let key = event.keystroke.key.as_str();
        if !matches!(key, "up" | "down" | "tab" | "enter" | "escape") {
            return;
        }
        let Some(entity) = weak_keys.upgrade() else {
            return;
        };
        let handled = entity.update(cx, |this, cx| {
            if key == "escape" {
                if this.preview_image.is_some() {
                    this.preview_image = None;
                    cx.notify();
                    return true;
                }
                if this.state.view.drawer == AiDrawerTab::Chat
                    && this.state.view.active_popup
                        != crate::app::ai::state::AiActivePopup::None
                {
                    this.state.view.active_popup = crate::app::ai::state::AiActivePopup::None;
                    cx.notify();
                    return true;
                }
                if this.state.view.drawer == AiDrawerTab::Chat
                    && this
                        .composer_input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                    && (!this.current_slash_items(cx).is_empty()
                        || !this.current_mention_items(cx).is_empty())
                {
                    this.completion_dismissed_input =
                        Some(this.composer_input.read(cx).value().to_string());
                    cx.notify();
                    return true;
                }
                return false;
            }
            if this.state.view.drawer != AiDrawerTab::Chat
                || !this
                    .composer_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            {
                return false;
            }
            let slash_items = this.current_slash_items(cx);
            if !slash_items.is_empty() {
                match key {
                    "up" => {
                        this.slash_selected_idx =
                            step_slash_index(this.slash_selected_idx, slash_items.len(), -1);
                        this.popover_scroll_handle
                            .scroll_to_item(this.slash_selected_idx);
                        cx.notify();
                    }
                    "down" => {
                        this.slash_selected_idx =
                            step_slash_index(this.slash_selected_idx, slash_items.len(), 1);
                        this.popover_scroll_handle
                            .scroll_to_item(this.slash_selected_idx);
                        cx.notify();
                    }
                    "tab" | "enter" => {
                        let idx = this.slash_selected_idx.min(slash_items.len() - 1);
                        let cmd = slash_items[idx].command.clone();
                        this.apply_slash_command(&cmd, window, cx);
                    }
                    _ => return false,
                }
                return true;
            }
            let mention_items = this.current_mention_items(cx);
            if mention_items.is_empty() {
                return false;
            }
            match key {
                "up" => {
                    this.slash_selected_idx =
                        step_slash_index(this.slash_selected_idx, mention_items.len(), -1);
                    this.popover_scroll_handle
                        .scroll_to_item(this.slash_selected_idx);
                    cx.notify();
                }
                "down" => {
                    this.slash_selected_idx =
                        step_slash_index(this.slash_selected_idx, mention_items.len(), 1);
                    this.popover_scroll_handle
                        .scroll_to_item(this.slash_selected_idx);
                    cx.notify();
                }
                "tab" | "enter" => {
                    let idx = this.slash_selected_idx.min(mention_items.len() - 1);
                    let token = mention_items[idx].token.clone();
                    this.apply_mention_candidate(&token, window, cx);
                }
                _ => return false,
            }
            true
        });
        if handled {
            cx.stop_propagation();
        }
    })
}

impl AiAssistantView {
    pub(in crate::app::ai) fn render_slash_floating_popover(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let slash_items = self.current_slash_items(cx);
        if slash_items.is_empty() {
            return None;
        }
        let view = cx.entity();
        Some(render_slash_popover(
            &slash_items,
            self.slash_selected_idx,
            &self.popover_scroll_handle,
            p,
            move |cmd, w, cx| {
                view.update(cx, |this, cx| this.apply_slash_command(&cmd, w, cx));
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_active_slash, step_slash_index};

    #[test]
    fn step_slash_index_wraps_around_bounds() {
        assert_eq!(step_slash_index(0, 0, 1), 0);
        assert_eq!(step_slash_index(0, 6, 1), 1);
        assert_eq!(step_slash_index(5, 6, 1), 0);
        assert_eq!(step_slash_index(0, 6, -1), 5);
        assert_eq!(step_slash_index(3, 6, -1), 2);
        assert_eq!(step_slash_index(99, 4, -1), 2);
    }

    #[test]
    fn extract_active_slash_supports_slash_after_mention_and_ignores_paths() {
        assert_eq!(extract_active_slash("/in"), Some((0, "/in")));
        assert_eq!(extract_active_slash("@local /in"), Some((7, "/in")));
        assert_eq!(extract_active_slash("/var/log"), None);
        assert_eq!(extract_active_slash("@local /inspect done"), None);
    }
}
