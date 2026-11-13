//! asciinema 终端交互录制脚本只读回放器模块。
//!
//! 设计说明（Why）：
//! - 实现初衷：为用户提供脱机浏览、审查与教学演示终端会话录像的能力，支持全功能
//!   时间轴跳转（Seek）、快进/快退、播放倍速调节以及一键循环重播；
//! - 核心优势：
//!   1. 直接复用 Transom 高性能 TerminalView 与仿真内核，100% 精确呈现 ANSI / 24-bit 颜色与 CJK 宽字符；
//!   2. 采用纯虚拟读写管道（I/O Sink），物理隔离外部网络与本地宿主，输入只读且安全；
//!   3. 采用 RIS 复位 + 增量事件流秒级快照重建技术，彻底解决终端状态机回退时的错乱与残影问题；
//!   4. 复用 GPUI-kit 官方原生 Slider 组件，保持全应用 UI 风格统一。

use crate::cast_player_io::{ChannelReader, format_duration};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::*;
use std::io::sink;
use std::path::PathBuf;
use std::time::Duration;
use transom_core::{TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalProfile};
use transom_terminal::{CastEvent, CastEventType, CastHeader, TerminalView};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 录像终端输出帧。
#[derive(Debug, Clone)]
struct CastOutputEvent {
    time: f64,
    data: Vec<u8>,
}

/// 只读终端会话录像播放器视图。
pub(crate) struct CastPlayerView {
    pub(crate) _path: PathBuf,
    pub(crate) title: String,
    pub(crate) header: CastHeader,
    output_events: Vec<CastOutputEvent>,
    total_duration: f64,
    current_time: f64,
    is_playing: bool,
    speed: f64,
    next_event_idx: usize,
    data_tx: flume::Sender<Vec<u8>>,
    terminal: Entity<TerminalView>,
    progress_slider: Entity<SliderState>,
    _slider_sub: Subscription,
    _playback_task: Option<Task<()>>,
    focus_handle: FocusHandle,
}

/// 录包回放器构造参数结构体。
pub(crate) struct CastPlayerParams {
    pub(crate) path: PathBuf,
    pub(crate) title: Option<String>,
    pub(crate) header: CastHeader,
    pub(crate) events: Vec<CastEvent>,
    pub(crate) profile: TerminalProfile,
}

impl CastPlayerView {
    /// 创建新的只读会话录像播放器。
    pub(crate) fn new(
        params: CastPlayerParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let CastPlayerParams {
            path,
            title,
            header,
            events,
            profile,
        } = params;
        let (data_tx, data_rx) = flume::bounded::<Vec<u8>>(256);
        let reader = Box::new(ChannelReader::new(data_rx));
        let writer = Box::new(sink());
        let (control, _) = TerminalControlHandle::channel(16);
        let io = TerminalIo::new(reader, writer, control);

        let terminal = create_playback_terminal(window, cx, io, profile);
        let output_events = collect_output_events(events);

        let total_duration = output_events.last().map_or(0.0, |e| e.time);
        let max_slider = if total_duration > 0.0 {
            total_duration as f32
        } else {
            1.0
        };

        let progress_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(max_slider)
                .step(0.1)
                .default_value(0.0)
        });

        let slider_sub =
            cx.subscribe(
                &progress_slider,
                |this, _, event: &SliderEvent, cx| match event {
                    SliderEvent::Change(SliderValue::Single(val)) => {
                        this.seek(*val as f64, cx);
                    }
                    SliderEvent::Release(SliderValue::Single(val)) => {
                        this.seek(*val as f64, cx);
                    }
                    _ => {}
                },
            );

        let display_title = title.unwrap_or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("session.cast")
                .to_string()
        });

        let mut player = Self {
            _path: path,
            title: display_title,
            header,
            output_events,
            total_duration,
            current_time: 0.0,
            is_playing: true,
            speed: 1.0,
            next_event_idx: 0,
            data_tx,
            terminal,
            progress_slider,
            _slider_sub: slider_sub,
            _playback_task: None,
            focus_handle: cx.focus_handle(),
        };

        player.schedule_tick(cx);
        player
    }

    /// 开始播放。
    pub(crate) fn play(&mut self, cx: &mut Context<Self>) {
        if self.current_time >= self.total_duration {
            self.seek(0.0, cx);
        }
        self.is_playing = true;
        self.schedule_tick(cx);
        cx.notify();
    }

    /// 暂停播放。
    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        self.is_playing = false;
        self._playback_task = None;
        cx.notify();
    }

    /// 切换播放/暂停。
    pub(crate) fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.is_playing {
            self.pause(cx);
        } else {
            self.play(cx);
        }
    }

    /// 跳转至目标时间点（Seek）。
    pub(crate) fn seek(&mut self, target_time: f64, cx: &mut Context<Self>) {
        let target = target_time.clamp(0.0, self.total_duration);
        self.current_time = target;

        // VT100 初始复位 (RIS) + 屏幕清除 + 历史行清理 + 光标归位
        let reset_bytes = b"\x1bc\x1b[2J\x1b[3J\x1b[H";
        let _ = self.data_tx.send(reset_bytes.to_vec());

        // 重放从 0 到目标时间的全部输出帧
        let mut chunk = Vec::new();
        let mut next_idx = self.output_events.len();
        for (idx, ev) in self.output_events.iter().enumerate() {
            if ev.time <= target {
                chunk.extend_from_slice(&ev.data);
            } else {
                next_idx = idx;
                break;
            }
        }
        if !chunk.is_empty() {
            let _ = self.data_tx.send(chunk);
        }
        self.next_event_idx = next_idx;
        cx.notify();
    }

    /// 快进指定秒数。
    pub(crate) fn step_forward(&mut self, secs: f64, cx: &mut Context<Self>) {
        self.seek(self.current_time + secs, cx);
    }

    /// 快退指定秒数。
    pub(crate) fn step_backward(&mut self, secs: f64, cx: &mut Context<Self>) {
        self.seek(self.current_time - secs, cx);
    }

    /// 循环切换播放倍速。
    pub(crate) fn cycle_speed(&mut self, cx: &mut Context<Self>) {
        self.speed = match self.speed {
            s if s < 0.9 => 1.0,
            s if s < 1.4 => 1.5,
            s if s < 1.9 => 2.0,
            s if s < 3.0 => 4.0,
            _ => 0.5,
        };
        cx.notify();
    }

    /// 重新从头开始播放。
    pub(crate) fn restart(&mut self, cx: &mut Context<Self>) {
        self.seek(0.0, cx);
        self.play(cx);
    }

    /// 启动播放帧时钟定时任务。
    fn schedule_tick(&mut self, cx: &mut Context<Self>) {
        if !self.is_playing {
            return;
        }

        let tick_duration = Duration::from_millis(30);
        self._playback_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(tick_duration).await;
                let active = this.update(cx, |player, cx| {
                    if !player.is_playing {
                        return false;
                    }
                    let dt = 0.030 * player.speed;
                    let next_time = (player.current_time + dt).min(player.total_duration);
                    player.current_time = next_time;

                    let mut send_bytes = Vec::new();
                    while player.next_event_idx < player.output_events.len() {
                        let ev = &player.output_events[player.next_event_idx];
                        if ev.time <= next_time {
                            send_bytes.extend_from_slice(&ev.data);
                            player.next_event_idx += 1;
                        } else {
                            break;
                        }
                    }
                    if !send_bytes.is_empty() {
                        let _ = player.data_tx.send(send_bytes);
                    }

                    if next_time >= player.total_duration {
                        player.is_playing = false;
                        cx.notify();
                        return false;
                    }
                    cx.notify();
                    true
                });

                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        }));
    }
}

impl CastPlayerView {
    /// 底部控制条：播放组与进度组横向拼接。
    fn render_control_bar(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("player-control-bar")
            .h(px(40.0))
            .px_3()
            .gap_2()
            .items_center()
            .bg(p.tab_bar)
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Top, p.divider_width)
            .child(self.render_playback_buttons(p, cx))
            .child(self.render_progress_controls(p, cx))
            .into_any_element()
    }

    /// 播放/后退/前进三个按钮。
    fn render_playback_buttons(&self, _p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let play_icon = if self.is_playing {
            AppIcon::Ui(UiIcon::PAUSE)
        } else {
            AppIcon::Ui(UiIcon::RUN)
        };
        let play_tooltip = if self.is_playing {
            rust_i18n::t!("recorder.pause").to_string()
        } else {
            rust_i18n::t!("recorder.play").to_string()
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(
                crate::widgets::IconButton::new("player-play-toggle", play_icon, play_tooltip)
                    .size(crate::widgets::IconButtonSize::Compact)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_play(cx);
                    })),
            )
            .child(
                crate::widgets::IconButton::new(
                    "player-backward-5s",
                    AppIcon::Ui(UiIcon::UNDO),
                    rust_i18n::t!("recorder.backward_5s").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.step_backward(5.0, cx);
                })),
            )
            .child(
                crate::widgets::IconButton::new(
                    "player-forward-5s",
                    AppIcon::Ui(UiIcon::REFRESH),
                    rust_i18n::t!("recorder.forward_5s").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.step_forward(5.0, cx);
                })),
            )
            .into_any_element()
    }

    /// 时间、进度滑块、倍速、重播与分辨率徽标。
    fn render_progress_controls(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let time_label = format!(
            "{} / {}",
            format_duration(self.current_time),
            format_duration(self.total_duration)
        );
        let speed = self.speed;
        let resolution_badge = format!(
            "{} · {}×{}",
            self.title, self.header.width, self.header.height
        );
        h_flex()
            .flex_1()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .font_family(".SystemUIFont")
                    .child(time_label),
            )
            .child(
                div()
                    .flex_1()
                    .px_2()
                    .child(Slider::new(&self.progress_slider)),
            )
            .child(
                Button::new("player-speed-btn")
                    .ghost()
                    .xsmall()
                    .label(format!("{speed:.1}x"))
                    .tooltip(rust_i18n::t!("recorder.speed").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cycle_speed(cx);
                    })),
            )
            .child(
                crate::widgets::IconButton::new(
                    "player-restart-btn",
                    AppIcon::Ui(UiIcon::REFRESH),
                    rust_i18n::t!("recorder.restart").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.restart(cx);
                })),
            )
            .child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(p.radius.sm)
                    .bg(p.muted)
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(resolution_badge),
            )
            .into_any_element()
    }
}

impl Render for CastPlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let current_time = self.current_time;

        // 刷新渲染帧时静默同步滑块位置
        self.progress_slider.update(cx, |slider, cx| {
            slider.set_value(current_time as f32, window, cx);
        });

        let control_bar = self.render_control_bar(&p, cx);

        v_flex()
            .id("cast-player-root")
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "space" => this.toggle_play(cx),
                    "left" => this.step_backward(5.0, cx),
                    "right" => this.step_forward(5.0, cx),
                    "r" => this.restart(cx),
                    _ => {}
                }
            }))
            .child(
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .child(self.terminal.clone()),
            )
            .child(control_bar)
    }
}

/// 创建只读回放终端：输出通过通道喂给 TerminalView。
fn create_playback_terminal(
    window: &mut Window,
    cx: &mut Context<CastPlayerView>,
    io: TerminalIo,
    profile: TerminalProfile,
) -> Entity<TerminalView> {
    cx.new(|cx| {
        TerminalView::new(
            window,
            cx,
            transom_terminal::TerminalViewParams {
                io,
                profile,
                interaction: TerminalInteraction::default(),
                key_filter: None,
                audit_path: None,
                // 回放是既成事实的录像，不再向会话注入集成脚本。
                shell_integration_enabled: false,
                // 录像中的 OSC 52 不是用户当前会话发起的实时请求，不能被设置
                // 中的默认允许策略放行，否则打开审计文件就可能静默覆盖剪贴板。
                remote_clipboard_allowed: false,
                clear_on_first_resize: false,
            },
        )
    })
}

/// 只保留输出事件并按时间排序，作为回放时间轴。
fn collect_output_events(events: Vec<CastEvent>) -> Vec<CastOutputEvent> {
    let mut output_events = Vec::with_capacity(events.len());
    for ev in events {
        if ev.event_type == CastEventType::Output {
            output_events.push(CastOutputEvent {
                time: ev.time,
                data: ev.data.into_bytes(),
            });
        }
    }
    output_events.sort_by(|a, b| a.time.total_cmp(&b.time));
    output_events
}
