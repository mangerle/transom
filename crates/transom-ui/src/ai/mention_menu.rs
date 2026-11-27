//! AI 输入框 `@` 提及目标（SSH 主机、Docker 容器引擎、本机）自动补全弹层。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InlineToken;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, DomainIcon, UiIcon};

use super::slash_menu::step_slash_index;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::search::any_field_matches;
use crate::theme::tokens::{space, typography};

/// 输入框 `@` 提及目标类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiMentionKind {
    Local,
    SshHost { connected: bool },
    ContainerEngine,
}

/// 输入框 `@` 提及候选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiMentionCandidate {
    pub(crate) token: String,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) kind: AiMentionKind,
}

/// 提取输入文本末尾正在输入的 `@query`（返回 `@` 起始字节偏移与查询词）。
///
/// 仅当 `@` 位于开头或前一个字符为空白、且之后不含空白时触发，避免误触邮箱地址。
pub(super) fn extract_active_mention(input: &str) -> Option<(usize, &str)> {
    let at_pos = input.rfind('@')?;
    if at_pos > 0 {
        let prev = input[..at_pos].chars().next_back()?;
        if !prev.is_whitespace() {
            return None;
        }
    }
    let query = &input[at_pos + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    Some((at_pos, query))
}

/// 根据末尾 `@query` 筛选提及候选项（大小写不敏感匹配 token/label/detail）。
pub(super) fn filter_mention_items(
    input: &str,
    candidates: &[AiMentionCandidate],
) -> Vec<AiMentionCandidate> {
    let Some((_, query)) = extract_active_mention(input) else {
        return Vec::new();
    };
    let q = query.trim().to_lowercase();
    candidates
        .iter()
        .filter(|c| any_field_matches(&q, [c.token.as_str(), c.label.as_str(), c.detail.as_str()]))
        .cloned()
        .collect()
}

fn mention_icon_and_badge(kind: AiMentionKind) -> (AppIcon, String, bool) {
    match kind {
        AiMentionKind::SshHost { connected: true } => (
            AppIcon::Ui(UiIcon::WEB_SERVER),
            rust_i18n::t!("ai.mention_badge_ssh_online").to_string(),
            true,
        ),
        AiMentionKind::SshHost { connected: false } => (
            AppIcon::Ui(UiIcon::WEB_SERVER),
            rust_i18n::t!("ai.mention_badge_ssh").to_string(),
            false,
        ),
        AiMentionKind::ContainerEngine => (
            AppIcon::Custom(CustomIcon::DOCKER),
            rust_i18n::t!("ai.mention_badge_docker").to_string(),
            false,
        ),
        AiMentionKind::Local => (
            AppIcon::Domain(DomainIcon::TERMINAL),
            rust_i18n::t!("ai.mention_badge_local").to_string(),
            false,
        ),
    }
}

/// 渲染输入框正上方的 `@` 提及目标补全弹层。
pub(super) fn render_mention_popover(
    items: &[AiMentionCandidate],
    selected_idx: usize,
    scroll_handle: &ScrollHandle,
    p: &Palette,
    on_pick: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let active_idx = selected_idx.min(items.len().saturating_sub(1));
    let mut list = v_flex()
        .id("ai-mention-list")
        .w_full()
        .max_h(px(240.0))
        .overflow_y_scroll()
        .track_scroll(scroll_handle);
    for (idx, item) in items.iter().enumerate() {
        let is_selected = idx == active_idx;
        let token = item.token.clone();
        let pick = on_pick.clone();
        let (icon, badge, online) = mention_icon_and_badge(item.kind);
        let mut row = h_flex()
            .id(("ai-mention-item", idx))
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::SM))
            .justify_between()
            .items_center()
            .rounded(p.radius.sm)
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| pick(token.clone(), w, cx))
            .child(
                h_flex()
                    .gap(rems(space::XS))
                    .items_center()
                    .flex_shrink_0()
                    .child(
                        div()
                            .text_color(if is_selected { p.primary } else { p.text.muted })
                            .child(AppIconElement::new(icon).small()),
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
                            .child(format!("@{}", item.token)),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(rems(space::XS))
                    .items_center()
                    .justify_end()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_right()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(item.detail.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(p.radius.sm)
                            .bg(p.secondary)
                            .text_size(rems(typography::CAPTION))
                            .text_color(if online { p.success } else { p.text.muted })
                            .child(badge),
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
    /// 获取当前输入框匹配的 `@` 提及候选项（跳过已转换为行内胶囊的片段）。
    pub(super) fn current_mention_items(&self, cx: &App) -> Vec<AiMentionCandidate> {
        let input = self.composer_input.read(cx);
        let current_input = input.value().to_string();
        if let Some((pos, _)) = extract_active_mention(&current_input)
            && input
                .tokens()
                .iter()
                .any(|span| span.range().start <= pos && pos < span.range().end)
        {
            return Vec::new();
        }
        filter_mention_items(&current_input, &self.state.composer.mention_candidates)
    }

    /// 将选中的 `@` 目标替换为行内胶囊芯片，保留在同一行内与光标对齐。
    pub(super) fn apply_mention_candidate(
        &mut self,
        token: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.slash_selected_idx = 0;
        let current = self.composer_input.read(cx).value().to_string();
        let Some((at_pos, _)) = extract_active_mention(&current) else {
            return;
        };
        let label = format!("@{token}");
        let inline = InlineToken::new(format!("mention:{label}"), label);
        let range = at_pos..current.len();
        self.composer_input.update(cx, |i, cx| {
            if i.replace_range_with_token(range, inline, window, cx)
                .is_ok()
            {
                i.insert(" ", window, cx);
            }
        });
        self.last_composer_len = self.composer_input.read(cx).value().len();
        cx.notify();
    }

    /// 若 `@` 提及弹层当前可见，则应用高亮项并返回 `true`。
    pub(super) fn try_apply_selected_mention(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let items = self.current_mention_items(cx);
        let Some(item) = items.get(self.slash_selected_idx.min(items.len().saturating_sub(1)))
        else {
            return false;
        };
        let token = item.token.clone();
        self.apply_mention_candidate(&token, window, cx);
        true
    }

    /// 渲染输入框正上方的 `@` 提及候选弹层。
    pub(super) fn render_mention_floating_popover(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let items = self.current_mention_items(cx);
        if items.is_empty() {
            return None;
        }
        let view = cx.entity();
        Some(render_mention_popover(
            &items,
            step_slash_index(self.slash_selected_idx, items.len(), 0),
            &self.popover_scroll_handle,
            p,
            move |token, w, cx| {
                view.update(cx, |this, cx| this.apply_mention_candidate(&token, w, cx));
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_active_mention, filter_mention_items};
    use crate::ai::state::{AiMentionCandidate, AiMentionKind};

    #[test]
    fn extracts_active_mention_only_after_whitespace_or_start() {
        assert_eq!(extract_active_mention("@"), Some((0, "")));
        assert_eq!(extract_active_mention("@prod"), Some((0, "prod")));
        assert_eq!(
            extract_active_mention("check @prod-01"),
            Some((6, "prod-01"))
        );
        assert_eq!(extract_active_mention("user@example.com"), None);
        assert_eq!(extract_active_mention("@prod done"), None);
    }

    #[test]
    fn filters_mention_candidates_case_insensitively() {
        let list = vec![
            AiMentionCandidate {
                token: "prod-01".to_owned(),
                label: "prod-01".to_owned(),
                detail: "root@10.0.0.1:22".to_owned(),
                kind: AiMentionKind::SshHost { connected: true },
            },
            AiMentionCandidate {
                token: "local".to_owned(),
                label: "local".to_owned(),
                detail: "本机终端".to_owned(),
                kind: AiMentionKind::Local,
            },
        ];
        assert_eq!(filter_mention_items("@", &list).len(), 2);
        assert_eq!(filter_mention_items("inspect @PROD", &list).len(), 1);
        assert_eq!(filter_mention_items("@10.0.0", &list).len(), 1);
        assert!(filter_mention_items("@unknown", &list).is_empty());
    }
}
