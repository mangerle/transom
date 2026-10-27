//! 命令历史建议：输入前缀时的候选浮层与采纳逻辑。
//!
//! 设计原理（Why）：
//! - 实现初衷：运维会话里同一批命令反复敲，fish/Warp/Termius 的"边打字边提示"
//!   能把三次按键的活干完；命令文本已由 OSC 133 双来源采到，这里只是把它用起来；
//! - 核心优势：视图只持有一个查询回调（`HistoryProvider`），不感知存储与会话，
//!   候选从哪来、留多少条都由应用侧决定，终端 crate 不必新增任何持久化依赖；
//! - 采纳只补全不执行：Tab 写差量字符、绝不追加回车，"回车=执行"这条终端里
//!   最深的直觉不被破坏；上下键完全不拦，shell 历史召回因此照旧可用；
//! - 代价与局限：本地只跟踪"我们发出去的按键"，shell 侧改行（↑ 调历史、Tab 补全、
//!   Ctrl-R 搜索）看不见，这类键一出现就作废跟踪并收起浮层——宁可这次不提示，
//!   也不给一条对不上的建议。浮层按格定位，只出现在焦点窗格内。

use crate::event::HistoryProvider;
use crate::view::TerminalView;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::*;

/// 一次最多展示的候选数：再多就超出"一眼挑中"的意义，也只是浪费绘制。
const MAX_SUGGESTIONS: usize = 6;

/// 触发提示的最短前缀字符数：一个字母能匹配上的东西太多，弹出来是干扰。
const MIN_PREFIX_CHARS: usize = 2;

/// 候选文本的最大宽度，避免超长命令把浮层撑出窗格。
const SUGGESTION_MAX_WIDTH: f32 = 520.0;

impl TerminalView {
    /// 挂上历史查询回调；不挂则本功能整体不生效，视图行为与从前一致。
    pub fn attach_history(&mut self, provider: HistoryProvider) {
        self.history_provider = Some(provider);
    }

    /// 采纳当前选中的候选：只写差量字符，其余交给 shell 回显。
    ///
    /// 返回 `false` 表示当前没有候选可采纳，调用方（Tab 键处理）应把按键
    /// 按原语义送进终端，不能吞掉用户的 Tab。
    pub(crate) fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(command) = self.suggestions.get(self.suggestion_index).cloned() else {
            return false;
        };
        let suffix = command
            .chars()
            .skip(self.input_line.chars().count())
            .collect::<String>();
        self.input_line = command;
        self.clear_suggestions();
        if !suffix.is_empty() {
            self.write_input(suffix.as_bytes());
            self.state.engine.lock().scroll_to_bottom();
        }
        cx.notify();
        true
    }

    /// 按当前输入行刷新候选。
    pub(crate) fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.history_provider.clone() else {
            return;
        };
        let prefix = self.input_line.clone();
        if prefix.chars().count() < MIN_PREFIX_CHARS {
            self.clear_suggestions();
            cx.notify();
            return;
        }
        let mut candidates = provider(&prefix);
        // 与输入行不吻合的候选一律剔除：即使查询实现有偏差，也不会
        // 补出半条别的命令。
        candidates.retain(|candidate| candidate.starts_with(&prefix) && *candidate != prefix);
        // 短的先出：前缀相同的补全里，最短的那条通常就是用户想要的。
        candidates.sort_by_key(|candidate| candidate.chars().count());
        candidates.dedup();
        candidates.truncate(MAX_SUGGESTIONS);
        self.suggestions = candidates;
        self.suggestion_index = 0;
        cx.notify();
    }

    pub(crate) fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestion_index = 0;
    }

    /// 输入行发生变化：`invalidate` 表示 shell 侧可能已改行，本地跟踪作废。
    pub(crate) fn note_input_changed(&mut self, invalidate: bool, cx: &mut Context<Self>) {
        if invalidate {
            self.input_line.clear();
            self.clear_suggestions();
            cx.notify();
            return;
        }
        self.refresh_suggestions(cx);
    }

    /// 渲染候选浮层；无候选或终端已退出时返回 `None`。
    pub(crate) fn render_suggestion_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.suggestions.is_empty() || self.exited {
            return None;
        }
        // 像素换算统一在 f32 域里做完再一次性转 Pixels：Pixels 的加减与标量乘
        // 混写会让表达式到处是 `px(...)`/`.into()`，读起来找不到几何意图。
        let (origin_x, origin_y, cell_width, cell_height, rows) = {
            let metrics = self.state.metrics.lock();
            (
                f32::from(metrics.origin.x),
                f32::from(metrics.origin.y),
                f32::from(metrics.cell_width),
                f32::from(metrics.cell_height),
                metrics.rows,
            )
        };
        let cursor = self.state.engine.lock().cursor_point();
        // 贴在光标所在行的下一行；已到最后一行时翻到上方，避免浮层跑出窗格。
        let below = (cursor.line.0 as usize) + 1 < rows;
        let top = if below {
            origin_y + cell_height * (cursor.line.0 as f32 + 1.0)
        } else {
            origin_y - cell_height
        };
        let left = origin_x + cell_width * (cursor.column.0.min(48) as f32).max(0.0);

        let theme = cx.theme();
        let items: Vec<AnyElement> = self
            .suggestions
            .iter()
            .enumerate()
            .map(|(index, command)| {
                let selected = index == self.suggestion_index;
                h_flex()
                    .id(ElementId::Name(
                        format!("history-suggestion-{index}").into(),
                    ))
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .rounded(theme.radius)
                    .bg(if selected {
                        theme.accent
                    } else {
                        theme.popover
                    })
                    .child(Icon::new(IconName::Clock).with_size(px(13.0)))
                    .child(
                        div()
                            .text_xs()
                            .max_w(px(SUGGESTION_MAX_WIDTH))
                            .overflow_hidden()
                            .child(command.clone()),
                    )
                    .into_any_element()
            })
            .collect();

        Some(
            v_flex()
                .id(ElementId::Name("history-suggestion-bar".into()))
                .absolute()
                .top(px(top))
                .left(px(left))
                .gap_1()
                .p_1()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_md()
                .children(items)
                .into_any_element(),
        )
    }
}
