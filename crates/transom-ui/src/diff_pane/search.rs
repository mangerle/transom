//! 差异视图内置轻量文本查找：关键字匹配、命中计数、`DiffDecoration` 高亮与滚动定位。
//!
//! 设计原理（Why）：
//! - 按 `similar::TextDiff` 视觉补丁顺序线性扫描两份文本：未变更行仅在修改侧登记一次，
//!   纯删除行登记在原始侧，新增行登记在修改侧，从根本上避免并排视图下同一未变更行被命中两次；
//! - 每次匹配产出的 `DiffLinePosition` 直接复用 `DiffState::scroll_to_line` 展开折叠行并滚动到位，
//!   命中高亮通过 `Diff::decorations` 声明式注入，不侵入组件库内部状态。

use crate::palette::Palette;
use gpui_kit::base::{Disableable, h_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::{DiffDecoration, DiffLinePosition, DiffLineRange, DiffSide};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Subscription, div, px,
};
use similar::{ChangeTag, TextDiff};
use transom_ui_icons::{AppIconElement, UiIcon};

use super::compute::{sanitize_diff_path, split_lines_stripped};
use super::types::DiffUiAction;

/// 差异面板内搜索框与匹配状态（随具体宿主的差异视图存活）。
#[derive(Default)]
pub(crate) struct DiffSearchState {
    pub open: bool,
    pub query: String,
    pub matches: Vec<DiffLinePosition>,
    pub current_idx: usize,
    pub input: Option<Entity<InputState>>,
    pub subscription: Option<Subscription>,
}

impl DiffSearchState {
    /// 依据当前差异文件内容重新计算所有匹配位置，并合理保留游标。
    pub(crate) fn recompute(&mut self, path: &str, old_text: &str, new_text: &str) {
        self.matches = find_diff_matches(path, old_text, new_text, &self.query);
        self.current_idx = if self.matches.is_empty() {
            0
        } else {
            self.current_idx.min(self.matches.len() - 1)
        };
    }

    /// 清除匹配结果（用于重置或关闭前清理）。
    pub(crate) fn clear_matches(&mut self) {
        self.matches.clear();
        self.current_idx = 0;
    }

    /// 步进到上一处或下一处匹配，返回目标行位置供 `DiffState::scroll_to_line` 定位。
    pub(crate) fn step(&mut self, forward: bool) -> Option<DiffLinePosition> {
        if self.matches.is_empty() {
            return None;
        }
        let len = self.matches.len();
        self.current_idx = if forward {
            (self.current_idx + 1) % len
        } else {
            (self.current_idx + len - 1) % len
        };
        self.matches.get(self.current_idx).cloned()
    }

    /// 当前激活匹配的行位置（如有）。
    pub(crate) fn current_position(&self) -> Option<DiffLinePosition> {
        self.matches.get(self.current_idx).cloned()
    }

    /// 生成供 `Diff::decorations` 使用的行背景高亮列表。
    pub(crate) fn build_decorations(&self, p: &Palette) -> Vec<DiffDecoration> {
        if !self.open || self.matches.is_empty() {
            return Vec::new();
        }
        let mut decs = Vec::with_capacity(self.matches.len());
        let active_bg = p.warning.opacity(0.42);
        let passive_bg = p.warning.opacity(0.18);
        for (idx, pos) in self.matches.iter().enumerate() {
            let color = if idx == self.current_idx {
                active_bg
            } else {
                passive_bg
            };
            decs.push(
                DiffDecoration::new(DiffLineRange::new(
                    pos.path().clone(),
                    pos.side(),
                    pos.line(),
                    pos.line(),
                ))
                .with_background(color),
            );
        }
        decs
    }
}

/// 按补丁视觉顺序在原/新两侧全文中查找大小写不敏感的匹配行位置。
pub(crate) fn find_diff_matches(
    file_path: &str,
    old_text: &str,
    new_text: &str,
    query: &str,
) -> Vec<DiffLinePosition> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let clean_path = sanitize_diff_path(file_path);
    let old_lines = split_lines_stripped(old_text);
    let new_lines = split_lines_stripped(new_text);
    let diff = TextDiff::from_slices(&old_lines, &new_lines);

    let mut matches = Vec::new();
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                if let Some(line) = new_lines.get(new_line - 1)
                    && line.to_lowercase().contains(&needle)
                {
                    matches.push(DiffLinePosition::new(
                        clean_path.clone(),
                        DiffSide::Modified,
                        new_line,
                    ));
                }
                old_line += 1;
                new_line += 1;
            }
            ChangeTag::Delete => {
                if let Some(line) = old_lines.get(old_line - 1)
                    && line.to_lowercase().contains(&needle)
                {
                    matches.push(DiffLinePosition::new(
                        clean_path.clone(),
                        DiffSide::Original,
                        old_line,
                    ));
                }
                old_line += 1;
            }
            ChangeTag::Insert => {
                if let Some(line) = new_lines.get(new_line - 1)
                    && line.to_lowercase().contains(&needle)
                {
                    matches.push(DiffLinePosition::new(
                        clean_path.clone(),
                        DiffSide::Modified,
                        new_line,
                    ));
                }
                new_line += 1;
            }
        }
    }
    matches
}

/// 渲染差异面板内置查找条所需的回调参数对象。
pub(crate) struct DiffSearchBarProps<'a> {
    pub id_prefix: &'static str,
    pub state: &'a DiffSearchState,
    pub p: &'a Palette,
    pub on_prev: DiffUiAction,
    pub on_next: DiffUiAction,
    pub on_close: DiffUiAction,
}

/// 渲染差异工具条下方紧贴的轻量查找子栏。
pub(crate) fn render_diff_search_bar(props: DiffSearchBarProps<'_>) -> Option<AnyElement> {
    if !props.state.open {
        return None;
    }
    let input = props.state.input.as_ref()?;
    let total = props.state.matches.len();
    let has_matches = total > 0;
    let count_text = if props.state.query.is_empty() {
        String::new()
    } else if has_matches {
        format!("{}/{}", props.state.current_idx + 1, total)
    } else {
        rust_i18n::t!("git.diff.search_no_matches").to_string()
    };

    let on_prev = props.on_prev;
    let on_next = props.on_next;
    let on_close_esc = props.on_close.clone();
    let on_close_btn = props.on_close;

    Some(
        h_flex()
            .id(format!("{}-search-bar", props.id_prefix))
            .w_full()
            .h(px(30.0))
            .flex_none()
            .px_2()
            .gap_1p5()
            .items_center()
            .bg(props.p.background)
            .border_color(props.p.divider(props.p.border))
            .border_b_1()
            .text_xs()
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape") {
                    cx.stop_propagation();
                    on_close_esc(window, cx);
                }
            })
            .child(div().w(px(220.0)).child(Input::new(input).xsmall()))
            .when(!count_text.is_empty(), |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(if has_matches {
                            props.p.muted_foreground
                        } else {
                            props.p.warning
                        })
                        .child(count_text),
                )
            })
            .child(
                Button::new(format!("{}-search-prev", props.id_prefix))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(UiIcon::CHEVRON_UP))
                    .disabled(!has_matches)
                    .accessibility_label(rust_i18n::t!("git.diff.search_prev").to_string())
                    .tooltip(rust_i18n::t!("git.diff.search_prev").to_string())
                    .on_click(move |_, window, cx| on_prev(window, cx)),
            )
            .child(
                Button::new(format!("{}-search-next", props.id_prefix))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(UiIcon::CHEVRON_DOWN))
                    .disabled(!has_matches)
                    .accessibility_label(rust_i18n::t!("git.diff.search_next").to_string())
                    .tooltip(rust_i18n::t!("git.diff.search_next").to_string())
                    .on_click(move |_, window, cx| on_next(window, cx)),
            )
            .child(div().flex_1())
            .child(
                Button::new(format!("{}-search-close", props.id_prefix))
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(UiIcon::CLOSE))
                    .accessibility_label(rust_i18n::t!("git.diff.search_close").to_string())
                    .tooltip(rust_i18n::t!("git.diff.search_close").to_string())
                    .on_click(move |_, window, cx| on_close_btn(window, cx)),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_matches_in_patch_order_across_deleted_equal_and_inserted_lines() {
        let old_text = "alpha\nold_token\nshared_token\nomega\n";
        let new_text = "alpha\nnew_token\nshared_token\nomega\n";

        let matches = find_diff_matches("src/lib.rs", old_text, new_text, "TOKEN");
        assert_eq!(matches.len(), 3);
        assert_eq!(
            matches[0],
            DiffLinePosition::new("src/lib.rs", DiffSide::Original, 2)
        );
        assert_eq!(
            matches[1],
            DiffLinePosition::new("src/lib.rs", DiffSide::Modified, 2)
        );
        assert_eq!(
            matches[2],
            DiffLinePosition::new("src/lib.rs", DiffSide::Modified, 3)
        );
    }

    #[test]
    fn search_state_steps_and_wraps_cleanly() {
        let mut state = DiffSearchState {
            open: true,
            query: "foo".to_string(),
            ..Default::default()
        };
        state.recompute("a.rs", "foo\nbar\n", "foo\nfoo\n");
        assert_eq!(state.matches.len(), 2);
        assert_eq!(state.current_idx, 0);

        let next = state.step(true);
        assert_eq!(
            next,
            Some(DiffLinePosition::new("a.rs", DiffSide::Modified, 2))
        );
        assert_eq!(state.current_idx, 1);

        let wrapped = state.step(true);
        assert_eq!(
            wrapped,
            Some(DiffLinePosition::new("a.rs", DiffSide::Modified, 1))
        );
        assert_eq!(state.current_idx, 0);

        let back_wrapped = state.step(false);
        assert_eq!(
            back_wrapped,
            Some(DiffLinePosition::new("a.rs", DiffSide::Modified, 2))
        );
        assert_eq!(state.current_idx, 1);
    }
}
