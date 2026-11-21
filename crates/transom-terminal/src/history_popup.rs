//! 终端输入补全：边输入边在光标下方浮出候选，↑↓ 选择、Tab / Enter 采纳、Esc 收起。
//!
//! 设计原理（Why）：
//! - 实现初衷：运维会话里同一批命令反复敲，fish/Warp/Termius 的"边打字边提示"
//!   能把三次按键的活干完；命令文本已由 OSC 133 双来源采到，候选（历史、片段）
//!   的汇集与词位匹配都在应用侧与 `transom-core`，视图只负责取词、展示与采纳；
//! - 核心优势：候选按"正在输入的词在命令行里的位置"给出——命令位整库搜索，
//!   参数位只接得上前文的写法，对不上就不弹，Tab 仍交回 shell 自己的路径补全；
//!   采纳前用终端网格（OSC 133 记录的输入行起点到光标）核对本地跟踪行，只在
//!   两者一致或只差回显尾部时才退格替换，比盲改字节多一道防线；
//! - 采纳只补全不执行：退掉正在输入的词、写上候选文本，绝不追加回车，
//!   "回车=执行"这条终端里最深的直觉不被破坏；
//! - 代价与局限：本地只跟踪"我们发出去的按键"，shell 侧改行（↑ 调历史、Tab 补全、
//!   Ctrl-R 搜索）看不见，这类键一出现就作废跟踪并收起浮层——宁可这次不提示，
//!   也不给一条对不上的候选。浮层按格定位，只出现在焦点窗格内。

use crate::engine::PromptInputState;
use crate::event::CompletionProvider;
use crate::view::TerminalView;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use rust_i18n::t;
use transom_core::{CompletionKind, echo_lagging, last_token, merged_command};

/// 浮层内容的最大宽度，避免超长命令把浮层撑出窗格。
const POPUP_MAX_WIDTH: f32 = 520.0;

impl TerminalView {
    /// 挂上补全候选查询回调；不挂则本功能整体不生效，视图行为与从前一致。
    pub fn attach_completion(&mut self, provider: CompletionProvider) {
        self.completion_provider = Some(provider);
    }

    /// 按当前输入行刷新候选：取行尾正在输入的词与它前面的文本，交给应用侧查询。
    pub(crate) fn refresh_completions(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.completion_provider.clone() else {
            return;
        };
        let line = self.input_line.clone();
        if line.is_empty() {
            self.close_completions();
            cx.notify();
            return;
        }
        // 前台程序运行中（vim / ssh / TUI）时输入的不是 shell 命令行，绝不弹候选。
        if self.state.engine.lock().prompt_input_state() == PromptInputState::Running {
            self.close_completions();
            cx.notify();
            return;
        }
        // 替换目标是行尾正在敲的那个词，它前面的文本决定这个词是命令还是参数。
        // `last_token` 按空白与分隔符切分，词是整行的字节后缀，按字节长度截前文安全。
        let token = last_token(&line).to_string();
        let before = line[..line.len() - token.len()].to_string();
        let hits = provider(&token, &before);
        if hits.is_empty() {
            self.close_completions();
        } else {
            self.completions = hits;
            self.completion_index = 0;
        }
        cx.notify();
    }

    /// 收起浮层。
    pub(crate) fn close_completions(&mut self) {
        self.completions.clear();
        self.completion_index = 0;
    }

    /// ↑↓ 移动选中项（循环）。
    pub(crate) fn move_completion(&mut self, down: bool, cx: &mut Context<Self>) {
        let len = self.completions.len();
        if len == 0 {
            return;
        }
        self.completion_index = if down {
            (self.completion_index + 1) % len
        } else {
            (self.completion_index + len - 1) % len
        };
        cx.notify();
    }

    /// 采纳当前选中的候选：退掉正在输入的词，换成完整文本，不追加回车。
    ///
    /// 返回 `false` 表示当前没有候选可采纳，调用方应把按键按原语义送进终端，
    /// 不能吞掉用户的 Tab / Enter。
    pub(crate) fn accept_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.completions.get(self.completion_index).cloned() else {
            return false;
        };
        let tracked = self.input_line.clone();
        let token = last_token(&tracked).to_string();
        let token_chars: Vec<char> = token.chars().collect();
        let tracked_chars: Vec<char> = tracked.chars().collect();
        let prefix_chars = tracked_chars.len() - token_chars.len();

        // 替换前先核对网格：OSC 133 记录的输入行起点到光标之间的内容，与本地
        // 跟踪一致（或只差回显尾部）才动字节；对不上说明 shell 侧改过行，
        // 宁可收起浮层放弃这次补全，也不盲删用户行里的字符。
        let fallback = tracked_chars[..prefix_chars].iter().collect::<String>();
        let verified = {
            let engine = self.state.engine.lock();
            match engine.prompt_input_state() {
                PromptInputState::Empty | PromptInputState::Edited => match engine
                    .grid_command_text()
                {
                    Some(grid) => {
                        let grid_chars: Vec<char> = grid.chars().collect();
                        let anchored = if token_chars.is_empty() {
                            // 词为空（刚敲完空格）时没有可核对的词，只能核对整行；
                            // 网格文本会裁掉行尾空白，两侧同样去尾再比。
                            grid.trim_end() == tracked.trim_end()
                        } else {
                            grid_chars.ends_with(&token_chars)
                        };
                        (anchored || echo_lagging(&grid_chars, &tracked_chars, token_chars.len()))
                            .then_some(fallback)
                    }
                    // 网格还没有输入行起点（B 标记未到）时退回本地跟踪口径，
                    // 与集成缺席时的历史行为保持一致。
                    None => Some(fallback),
                },
                // 集成标记不足（Unknown）或运行中（理论不可达）时退回本地跟踪口径。
                PromptInputState::Unknown | PromptInputState::Running => Some(fallback),
            }
        };
        let Some(prefix) = verified else {
            self.close_completions();
            cx.notify();
            return true;
        };
        // 前缀末尾已写过候选命令开头的若干词时（如 `sudo apt ` + `sudo apt install …`）
        // 不重复，否则会拼出 `sudo apt sudo apt …`。
        let merged = merged_command(&prefix, &item.command);
        // 退格按字符数算：readline 里每个 0x7f 删一个字符，多字节字符不能按字节计。
        let mut bytes = vec![0x7f; token_chars.len()];
        bytes.extend_from_slice(merged.as_bytes());
        self.close_completions();
        if !bytes.is_empty() {
            self.state.engine.lock().note_prompt_input_edited();
            self.write_input(&bytes);
        }
        self.input_line = format!("{prefix}{merged}");
        self.state.engine.lock().scroll_to_bottom();
        cx.notify();
        true
    }

    /// 输入行发生变化：`invalidate` 表示 shell 侧可能已改行，本地跟踪作废。
    pub(crate) fn note_input_changed(&mut self, invalidate: bool, cx: &mut Context<Self>) {
        if invalidate {
            self.input_line.clear();
            self.close_completions();
            cx.notify();
            return;
        }
        self.refresh_completions(cx);
    }

    /// 渲染候选浮层；无候选或终端已退出时返回 `None`。
    pub(crate) fn render_completion_popup(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.completions.is_empty() || self.exited {
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
            .completions
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selected = index == self.completion_index;
                let badge = match item.kind {
                    CompletionKind::Flag => t!("terminal.completion.badge_option").to_string(),
                    CompletionKind::History => t!("terminal.completion.badge_history").to_string(),
                    CompletionKind::Snippet => t!("terminal.completion.badge_snippet").to_string(),
                    CompletionKind::Command => t!("terminal.completion.badge_command").to_string(),
                };
                // 选项候选补的就是它自己，第二列改放它来自哪条命令，免得两列重复；
                // 历史候选的名称就是整条命令，第二列再放一遍只是占地方。
                let detail = match item.kind {
                    CompletionKind::Flag => Some(item.note.as_str()),
                    CompletionKind::History => None,
                    CompletionKind::Snippet | CompletionKind::Command => {
                        Some(item.command.as_str())
                    }
                };
                h_flex()
                    .id(ElementId::Name(format!("completion-row-{index}").into()))
                    .min_w_0()
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
                    // 历史候选的名称是整条命令行，可能长过浮层宽度：让它参与收缩
                    // 并省略，其余候选保持按名称宽度占位。
                    .child({
                        let name = div().text_xs().child(item.name.clone());
                        if item.kind == CompletionKind::History {
                            name.flex_1().min_w_0().truncate()
                        } else {
                            name.flex_shrink_0()
                        }
                    })
                    .children(detail.map(|text| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(text.to_string())
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(badge),
                    )
                    .into_any_element()
            })
            .collect();

        Some(
            v_flex()
                .id(ElementId::Name("completion-popup".into()))
                .absolute()
                .top(px(top))
                .left(px(left))
                .max_w(px(POPUP_MAX_WIDTH))
                .overflow_hidden()
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
