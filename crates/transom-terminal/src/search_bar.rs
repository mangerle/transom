//! 终端悬浮检索栏：搜索栏渲染、查询同步、命中导航与刻度带点击。
//!
//! 检索栏刻意做成终端层的**兄弟节点**而不是子节点：终端根 div 上挂了
//! `on_key_down`（把按键编码成控制序列写进 PTY），若搜索输入框位于其子树内，
//! 按键会沿焦点路径冒泡到该处理器，导致"在搜索框里打字"同时被送进 Shell。

use crate::search::SearchStatus;
use crate::view::TerminalView;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

/// 搜索输入框宽度：短到不遮挡正文，长到能看清常见日志关键字。
const SEARCH_BAR_INPUT_WIDTH: Pixels = px(240.0);
/// 检索栏与内容区上边界的距离。
const SEARCH_BAR_OFFSET_TOP: Pixels = px(8.0);
/// 检索栏左边距右边缘的距离，落在刻度带内侧。
const SEARCH_BAR_OFFSET_RIGHT: Pixels = px(18.0);

impl TerminalView {
    /// 呼出/收起检索栏，由工作区快捷键转发到当前窗格。
    pub fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_open {
            self.close_search(window, cx);
        } else {
            self.open_search(window, cx);
        }
    }

    /// 打开检索栏；已有选区时预填成查询词，对齐"选中什么就找什么"的习惯。
    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefill = self
            .state
            .engine
            .lock()
            .selection_text()
            .and_then(|text| crate::search::prefill_from_selection(Some(text.as_str())));
        let input = self
            .search_input
            .get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder("搜索终端输出"))
            })
            .clone();
        // 输入事件驱动同步，而不是每帧去读输入框：既省掉一次锁与字符串拷贝，
        // 也保证不依赖"渲染期 read 恰好登记了依赖"这种隐性行为。
        if self._search_subscription.is_none() {
            self._search_subscription =
                Some(cx.subscribe(&input, |view, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.sync_search_query(cx);
                        cx.notify();
                    }
                }));
        }
        if let Some(prefill) = prefill {
            input.update(cx, |state, cx| state.set_value(prefill, window, cx));
        }
        self.search_open = true;
        self.sync_search_query(cx);
        input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.state.engine.lock().clear_search();
        let focus = self.focus_handle().clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// 把输入框内容同步进引擎；查询未变化时引擎内部短路，不会作废快照。
    pub(crate) fn sync_search_query(&mut self, cx: &mut Context<Self>) {
        if !self.search_open {
            return;
        }
        let Some(input) = self.search_input.as_ref() else {
            return;
        };
        let query = input.read(cx).value();
        self.state
            .engine
            .lock()
            .set_search_query(query.as_ref(), self.search_regex_mode);
    }

    pub(crate) fn on_search_enter(
        &mut self,
        action: &Enter,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_open {
            return;
        }
        // Shift+Enter 反向查找，与多数编辑器查找栏的口径一致。
        self.state.engine.lock().search_select(!action.shift);
        cx.notify();
    }

    pub(crate) fn on_search_escape(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_open {
            self.close_search(window, cx);
        }
    }

    /// 检索栏打开时，鼠标落在右侧刻度带上的处理：按比例跳到最近的命中。
    ///
    /// 返回 `true` 表示这次点击已被检索消费，不应再触发文本选择。
    pub(crate) fn search_strip_click(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.search_open {
            return false;
        }
        let Some(strip) = self.state.metrics.lock().search_strip else {
            return false;
        };
        if !strip.contains(&position) || f32::from(strip.size.height) <= 0.0 {
            return false;
        }
        let fraction =
            (f32::from(position.y) - f32::from(strip.origin.y)) / f32::from(strip.size.height);
        self.state.engine.lock().search_select_by_fraction(fraction);
        // 跳转后把焦点还给输入框，否则下一次回车会落回终端而不是继续查找。
        if let Some(input) = self.search_input.as_ref() {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
        true
    }

    /// 按钮路径的上一个/下一个命中。
    fn search_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.state.engine.lock().search_select(forward);
        cx.notify();
    }

    fn toggle_search_regex(&mut self, cx: &mut Context<Self>) {
        self.search_regex_mode = !self.search_regex_mode;
        // 模式变化会让引擎重新编译自动机，必须立刻同步一次，否则按钮亮着却仍按旧模式匹配。
        self.sync_search_query(cx);
        cx.notify();
    }

    /// 渲染悬浮检索栏；未打开时返回 `None`。
    pub(crate) fn render_search_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.search_open {
            return None;
        }
        let input = self.search_input.clone()?;
        let status = self.state.engine.lock().search_status();
        let label = search_status_text(&status);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let popover = theme.popover;
        let regex_mode = self.search_regex_mode;
        let entity = cx.entity().clone();

        let bar = h_flex()
            .id("terminal-search-bar")
            .absolute()
            .top(SEARCH_BAR_OFFSET_TOP)
            .right(SEARCH_BAR_OFFSET_RIGHT)
            .gap_1()
            .items_center()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(popover)
            .child(Icon::new(IconName::Search).small())
            .child(div().w(SEARCH_BAR_INPUT_WIDTH).child(Input::new(&input)))
            .child(
                div()
                    .text_xs()
                    .max_w(px(220.0))
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(muted)
                    .child(label),
            )
            .child(search_bar_button(
                "terminal-search-regex",
                IconName::Regex,
                "正则表达式",
                regex_mode,
                SearchBarButton::Regex,
                entity.clone(),
            ))
            .child(search_bar_button(
                "terminal-search-prev",
                IconName::ChevronUp,
                "上一个匹配 (Shift+Enter)",
                false,
                SearchBarButton::Prev,
                entity.clone(),
            ))
            .child(search_bar_button(
                "terminal-search-next",
                IconName::ChevronDown,
                "下一个匹配 (Enter)",
                false,
                SearchBarButton::Next,
                entity.clone(),
            ))
            .child(search_bar_button(
                "terminal-search-close",
                IconName::X,
                "关闭搜索 (Esc)",
                false,
                SearchBarButton::Close,
                entity,
            ))
            // 点击搜索栏自身不穿透到终端层，否则会抢回焦点并清掉选区。
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            });
        Some(bar.into_any_element())
    }
}

/// 检索栏按钮语义。用枚举而不是在回调里比较元素 id，避免拼错 id 就静默失灵。
#[derive(Clone, Copy)]
enum SearchBarButton {
    Regex,
    Prev,
    Next,
    Close,
}

/// 构造检索栏上的图标按钮；`selected` 只用于正则开关的按压态。
fn search_bar_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    selected: bool,
    action: SearchBarButton,
    entity: Entity<TerminalView>,
) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .selected(selected)
        .tooltip(tooltip)
        .on_click(move |_, window, cx| {
            entity.update(cx, |view, cx| match action {
                SearchBarButton::Regex => view.toggle_search_regex(cx),
                SearchBarButton::Prev => view.search_step(false, cx),
                SearchBarButton::Next => view.search_step(true, cx),
                SearchBarButton::Close => view.close_search(window, cx),
            });
        })
}

/// 检索栏状态文本：命中计数、无匹配、截断或正则错误。
fn search_status_text(status: &SearchStatus) -> String {
    if let Some(error) = &status.error {
        return format!("正则无效：{error}");
    }
    if !status.has_query {
        return "输入关键词开始搜索".to_string();
    }
    if status.stale {
        // 快照过期时明确告知为什么计数不再变，而不是让界面看起来"卡住没反应"。
        return "输出中，稍后自动更新".to_string();
    }
    if status.total == 0 {
        return "没有匹配项".to_string();
    }
    let suffix = if status.truncated { "+" } else { "" };
    if status.ordinal == 0 {
        return format!("共 {}{suffix} 处匹配", status.total);
    }
    format!("{} / {}{suffix}", status.ordinal, status.total)
}
