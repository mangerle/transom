mod block;
mod input;
mod menu;
mod paint;
mod tasks;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use input::build_custom_keys;
pub use types::KeyFilter;
pub(crate) use types::{
    CONTEXT, LayoutMetrics, PreparedTerminal, READ_BUFFER_BYTES, READ_CHANNEL_CAPACITY,
    SEARCH_TICK_PIXELS, TerminalRenderState,
};

use crate::engine::TerminalEngine;
use crate::event::{HistoryProvider, TerminalEvent};
use crate::input::{focus_bytes, paste_bytes};
use crate::osc::CommandTracker;
use crate::record::RecorderHandle;
use crate::renderer::TerminalRenderer;
use alacritty_terminal::event::Event;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use transom_core::{
    BlockViewSettings, TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalProfile,
    TerminalSize,
};

actions!(transom_terminal, [SendTab, SendBacktab]);

pub struct TerminalView {
    /// 终端渲染与引擎核心状态聚合容器。
    pub(crate) state: Arc<TerminalRenderState>,
    profile: TerminalProfile,
    interaction: TerminalInteraction,
    focus_handle: FocusHandle,
    input_tx: flume::Sender<Vec<u8>>,
    control: TerminalControlHandle,
    event_rx: flume::Receiver<Event>,
    key_filter: Option<KeyFilter>,
    pub(crate) ime_text: String,
    pub(crate) ime_marked_range: Option<Range<usize>>,
    pub(crate) dragging_selection: bool,
    pub(crate) mouse_pressed: bool,
    pub(crate) exited: bool,
    /// 当前闪烁相位；关闭闪烁或失焦时始终保持为 `true`。
    cursor_blink_on: bool,
    /// 像素滚动的未凑整余数（单位：行）。触摸板逐事件位移常小于一行，
    /// 不累积的话 round 会把小于半行的事件全部舍成 0，平滑滚动失效。
    scroll_pixel_remainder: f32,
    /// 悬浮检索栏是否展开。
    pub(crate) search_open: bool,
    /// 检索输入框状态；首次展开时创建，收起后保留上次的查询词。
    pub(crate) search_input: Option<Entity<InputState>>,
    /// 命令生命周期归并：把 OSC 133 的零散标记算成"一条命令跑了多久"。
    command_tracker: CommandTracker,
    /// 命令历史候选查询；未挂接时建议功能整体不生效。
    pub(crate) history_provider: Option<HistoryProvider>,
    /// 本地跟踪的当前输入行，只用于取建议前缀。
    pub(crate) input_line: String,
    /// 当前候选列表与选中位置（Tab 逐个采纳，不抢上下键）。
    pub(crate) suggestions: Vec<String>,
    pub(crate) suggestion_index: usize,
    /// 是否把查询词当正则表达式解析（与引擎里的快照同步）。
    pub(crate) search_regex_mode: bool,
    /// 检索输入框的事件订阅；随实体存活，收起搜索栏也不释放（下次复用同一输入框）。
    pub(crate) _search_subscription: Option<Subscription>,
    /// 语义块视图配置；`None` 或 `enabled == false` 时走经典连续流。
    block_view: Option<BlockViewSettings>,
    /// 当前悬浮的命令块与所在可见行；`None` 表示不在块上。
    hover_block: Option<(u64, usize)>,
    /// 已解析的自定义按键映射；键盘热路径只做遍历匹配。
    custom_keys: Vec<(KeybindingKeystroke, Vec<u8>)>,
    _reader_task: Task<()>,
    _blink_task: Task<()>,
    _focus_subscriptions: Vec<Subscription>,
    recorder: RecorderHandle,
}

impl TerminalView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        io: TerminalIo,
        profile: TerminalProfile,
        interaction: TerminalInteraction,
        key_filter: Option<KeyFilter>,
        audit_path: Option<std::path::PathBuf>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("tab", SendTab, Some(CONTEXT)),
            KeyBinding::new("shift-tab", SendBacktab, Some(CONTEXT)),
        ]);

        let focus_handle = cx.focus_handle().tab_stop(false);
        let (event_tx, event_rx) = flume::bounded(256);
        let renderer = TerminalRenderer::new(&profile, window, cx);
        let engine = TerminalEngine::new(TerminalSize::new(80, 24), &profile, event_tx.clone());
        let state = Arc::new(TerminalRenderState::new(engine, renderer));
        let custom_keys = build_custom_keys(&profile.key_bindings);

        let recorder = RecorderHandle::new();
        let TerminalIo {
            reader,
            writer,
            control,
        } = io;
        let (input_tx, reader_task) =
            Self::spawn_io_tasks(reader, writer, audit_path, &recorder, &event_tx, &state, cx);

        let on_focus = cx.on_focus(&focus_handle, window, |view, _, cx| {
            let mode = view.state.engine.lock().mode();
            if let Some(bytes) = focus_bytes(mode, true) {
                view.write_input(&bytes);
            }
            view.cursor_blink_on = true;
            cx.notify();
        });
        let on_blur = cx.on_blur(&focus_handle, window, |view, _, cx| {
            let mode = view.state.engine.lock().mode();
            if let Some(bytes) = focus_bytes(mode, false) {
                view.write_input(&bytes);
            }
            view.cursor_blink_on = true;
            cx.notify();
        });

        // 闪烁只驱动光标相位，不触碰终端内容；失焦或关闭闪烁时定时器保持静默。
        let blink_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let updated = this.update_in(cx, |view, window, cx| {
                    view.tick_cursor_blink(window, cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        Self {
            state,
            profile,
            interaction,
            focus_handle,
            input_tx,
            control,
            event_rx,
            key_filter,
            ime_text: String::new(),
            ime_marked_range: None,
            dragging_selection: false,
            mouse_pressed: false,
            exited: false,
            cursor_blink_on: true,
            scroll_pixel_remainder: 0.0,
            command_tracker: CommandTracker::default(),
            history_provider: None,
            input_line: String::new(),
            suggestions: Vec::new(),
            suggestion_index: 0,
            search_open: false,
            search_input: None,
            search_regex_mode: false,
            _search_subscription: None,
            block_view: None,
            hover_block: None,
            custom_keys,
            _reader_task: reader_task,
            _blink_task: blink_task,
            _focus_subscriptions: vec![on_focus, on_blur],
            recorder,
        }
    }

    /// 从设置重载当前终端的触发器规则。
    pub fn reload_triggers(&mut self, settings: &transom_core::TriggerSettings) {
        self.state.trigger_engine.lock().reload(settings);
    }

    fn tick_cursor_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blinking = self.focus_handle.is_focused(window) && self.profile.cursor_blink && {
            let engine = self.state.engine.lock();
            let style = engine.term().cursor_style();
            style.blinking
                && style.shape != CursorShape::Hidden
                && engine.mode().contains(TermMode::SHOW_CURSOR)
        };
        if blinking {
            self.cursor_blink_on = !self.cursor_blink_on;
            cx.notify();
        } else if !self.cursor_blink_on {
            self.cursor_blink_on = true;
            cx.notify();
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// 热应用全局外观偏好：字体族、中文字体族、字号、行高与关键词高亮规则。
    ///
    /// 渲染器是长生命周期缓存，改设置必须显式失效：这里置脏标记，下一帧
    /// prepaint（持有窗口文本系统）按新 profile 重建度量与编译规则。调色板、
    /// scrollback 与光标外观不在这里覆盖，那些仍归会话自己的 profile，
    /// 避免全局偏好悄悄吃掉会话级配置。
    /// 网格列数会随字号变化，由下一帧 prepaint 统一走既有的 resize + PTY 同步通路。
    pub fn apply_appearance(&mut self, profile: &TerminalProfile, cx: &mut Context<Self>) {
        self.profile.font_family = profile.font_family.clone();
        self.profile.cjk_font_family = profile.cjk_font_family.clone();
        self.profile.font_size = profile.font_size;
        self.profile.line_height = profile.line_height;
        self.profile.highlight = profile.highlight.clone();
        self.profile.alternate_scroll = profile.alternate_scroll;
        // 终端配色同样全局生效：界面上没有会话级配色入口，保存设置后活窗格
        // 立即换色；运行期程序经 OSC 4/10/11 的动态改色走另一条运行时表，
        // 不受此静态基线覆盖影响。
        self.profile.theme_name = profile.theme_name.clone();
        self.profile.palette = profile.palette.clone();
        if self.profile.key_bindings != profile.key_bindings {
            self.profile.key_bindings = profile.key_bindings.clone();
            self.custom_keys = build_custom_keys(&self.profile.key_bindings);
        }
        // 背景透明度参与渲染：折算成背景色的 alpha，透出下层应用背景。
        self.profile.background_transparency = profile.background_transparency;
        self.state.renderer_dirty.store(true, Ordering::Release);
        cx.notify();
    }

    /// 查找按键组合对应的自定义字节序列。
    fn custom_key_sequence(&self, keystroke: &Keystroke) -> Option<Vec<u8>> {
        self.custom_keys
            .iter()
            .find(|(binding, _)| keystroke.should_match(binding))
            .map(|(_, bytes)| bytes.clone())
    }

    pub(crate) fn write_input(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        // 输入瞬间强制点亮光标，避免用户打字时恰好处于熄灭相位。
        self.cursor_blink_on = true;
        self.recorder.record_input(bytes);
        match self.input_tx.try_send(bytes.to_vec()) {
            Ok(()) => true,
            // 256 深度对人速按键与整批粘贴绰绰有余，打满只可能是写入线程
            // 卡死：此时丢掉本批输入并继续存活判定，比在 UI 线程阻塞等待好。
            Err(flume::TrySendError::Full(_)) => {
                log::warn!("终端输入队列已满，本批输入被丢弃");
                true
            }
            Err(flume::TrySendError::Disconnected(_)) => false,
        }
    }

    /// 获取录制器句柄。
    pub fn recorder(&self) -> &RecorderHandle {
        &self.recorder
    }

    /// 查询当前窗格是否正在录制。
    pub fn is_recording(&self) -> bool {
        self.recorder.is_recording()
    }

    /// 开始录制当前终端会话到 `.cast` 文件。
    pub fn start_recording(
        &mut self,
        path: std::path::PathBuf,
        title: Option<String>,
    ) -> std::io::Result<()> {
        let term = self.state.engine.lock();
        let cols = term.term().grid().columns();
        let rows = term.term().grid().screen_lines();
        drop(term);
        self.recorder.start(path, cols as u16, rows as u16, title)
    }

    /// 停止录制并返回文件路径。
    pub fn stop_recording(&mut self) -> Option<std::path::PathBuf> {
        self.recorder.stop()
    }

    /// 将程序化文本写入终端输入通道。
    ///
    /// 与键盘输入复用同一条有界队列，保证初始化命令按调用顺序到达 Shell；
    /// 返回 `false` 表示终端写入线程已退出（连接已断开）。
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let sent = self.write_input(text.as_bytes());
        if sent {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
        }
        sent
    }

    /// 按粘贴语义把一段文本写入终端输入通道，可选在末尾补回车执行。
    ///
    /// 多行脚本使用终端当前的括号粘贴模式：远端 shell 会把整段文本当作一次
    /// 粘贴，而不是每读到一行就提前执行；对端未开启括号粘贴时退回原始字节。
    /// 与键盘输入复用同一条有界队列，返回 `false` 表示终端写入线程已退出。
    pub fn send_paste(&mut self, text: &str, execute: bool, cx: &mut Context<Self>) -> bool {
        let mode = self.state.engine.lock().mode();
        let mut bytes = paste_bytes(mode, text);
        if execute {
            bytes.push(b'\r');
        }
        let sent = self.write_input(&bytes);
        if sent {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
        }
        sent
    }

    fn mark_exited(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            cx.emit(TerminalEvent::Exit);
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.state.engine.lock().selection_text();
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(TerminalEvent::ClipboardStored);
            true
        } else {
            false
        }
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        let mode = self.state.engine.lock().mode();
        let bytes = paste_bytes(mode, &text);
        if self.write_input(&bytes) {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
            true
        } else {
            false
        }
    }

    fn hit_test_screen_cell(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let metrics = *self.state.metrics.lock();
        if metrics.cols == 0
            || metrics.rows == 0
            || metrics.cell_width <= px(0.0)
            || metrics.cell_height <= px(0.0)
        {
            return None;
        }
        let x = position.x - metrics.origin.x;
        let y = position.y - metrics.origin.y;
        if x < px(0.0) || y < px(0.0) {
            return None;
        }
        let col = (f32::from(x) / f32::from(metrics.cell_width)).floor() as usize;
        let row = (f32::from(y) / f32::from(metrics.cell_height)).floor() as usize;
        if col >= metrics.cols || row >= metrics.rows {
            return None;
        }
        Some((row, col))
    }

    fn hit_test_grid_point(&self, position: Point<Pixels>) -> Option<AlacPoint> {
        let (row, col) = self.hit_test_screen_cell(position)?;
        // 折叠模式下可见行与网格行不再一一对应：命中必须查行映射，
        // 否则选择会落到被隐藏的输出区或错位的行上。
        if let Some(layout) = self.state.block_layout.lock().clone() {
            return layout
                .line_at(row)
                .map(|line| AlacPoint::new(line, Column(col)));
        }
        let offset = self.state.engine.lock().display_offset() as i32;
        Some(AlacPoint::new(Line(row as i32 - offset), Column(col)))
    }
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.process_events(cx);

        let right_click = self.interaction.right_click_behavior();
        let menu_entity = cx.entity();
        let layer = div()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_send_tab))
            .on_action(cx.listener(Self::on_action_send_backtab))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(self.render_canvas(cx));
        let layer = Self::wrap_context_menu(layer, menu_entity, right_click);

        let search_bar = self.render_search_bar(cx);
        let block_actions = self.render_block_actions(cx);
        // 拖放高亮用主色调半透明蒙色：不改布局、不依赖主题里的具体色值，
        // 深浅色终端下都能看清"松手就落在这里"。
        let drop_tint = cx.theme().primary.opacity(0.12);
        let entity = cx.entity().clone();
        // 检索栏与终端层平级：放在终端层子树里会让输入框的按键沿焦点路径冒泡到
        // `on_key_down`，敲搜索词的同时也把字符送进了 Shell。
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .on_action(cx.listener(Self::on_search_enter))
            .on_action(cx.listener(Self::on_search_escape))
            // 外部文件拖放：命中区域就是网格本身，视图只投递事件，
            // 要不要接收、传到哪儿由应用层按会话类型决定。
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
            .on_drop(move |paths: &ExternalPaths, _window, cx| {
                let dropped = paths.paths().to_vec();
                if dropped.is_empty() {
                    return;
                }
                entity.update(cx, |_, cx| cx.emit(TerminalEvent::FilesDropped(dropped)));
            })
            .child(layer)
            .children(block_actions)
            .children(self.render_suggestion_bar(cx))
            .children(search_bar)
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.control.close();
    }
}
