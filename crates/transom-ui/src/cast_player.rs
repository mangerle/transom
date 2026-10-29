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

use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::*;
use std::io::{Cursor, Read, sink};
use std::path::PathBuf;
use std::time::Duration;
use transom_core::{TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalProfile};
use transom_terminal::{CastEvent, CastEventType, CastHeader, TerminalView};

/// 管道流式读取器：将 flume 接收端适配为标准 Read trait。
struct ChannelReader {
    rx: flume::Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
}

impl ChannelReader {
    fn new(rx: flume::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            current: Cursor::new(Vec::new()),
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let n = self.current.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            match self.rx.recv() {
                Ok(chunk) => {
                    self.current = Cursor::new(chunk);
                }
                Err(_) => return Ok(0),
            }
        }
    }
}

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

        let terminal = cx.new(|cx| {
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
                },
            )
        });

        // 提取全部输出事件
        let mut output_events = Vec::new();
        for ev in events {
            if ev.event_type == CastEventType::Output {
                output_events.push(CastOutputEvent {
                    time: ev.time,
                    data: ev.data.into_bytes(),
                });
            }
        }
        output_events.sort_by(|a, b| a.time.total_cmp(&b.time));

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

impl Render for CastPlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let current_time = self.current_time;
        let total_duration = self.total_duration;
        let is_playing = self.is_playing;
        let speed = self.speed;

        // 刷新渲染帧时静默同步滑块位置
        self.progress_slider.update(cx, |slider, cx| {
            slider.set_value(current_time as f32, window, cx);
        });

        let play_icon = if is_playing {
            IconName::Pause
        } else {
            IconName::Play
        };
        let play_tooltip = if is_playing {
            rust_i18n::t!("recorder.pause").to_string()
        } else {
            rust_i18n::t!("recorder.play").to_string()
        };

        let time_label = format!(
            "{} / {}",
            format_duration(current_time),
            format_duration(total_duration)
        );

        let resolution_badge = format!(
            "{} · {}×{}",
            self.title, self.header.width, self.header.height
        );

        let control_bar = h_flex()
            .id("player-control-bar")
            .h(px(40.0))
            .px_3()
            .gap_2()
            .items_center()
            .bg(p.tab_bar)
            .border_t_1()
            .border_color(p.border)
            .child(
                Button::new("player-play-toggle")
                    .ghost()
                    .xsmall()
                    .icon(play_icon)
                    .tooltip(play_tooltip)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_play(cx);
                    })),
            )
            .child(
                Button::new("player-backward-5s")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCcw)
                    .tooltip(rust_i18n::t!("recorder.backward_5s").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_backward(5.0, cx);
                    })),
            )
            .child(
                Button::new("player-forward-5s")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCw)
                    .tooltip(rust_i18n::t!("recorder.forward_5s").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_forward(5.0, cx);
                    })),
            )
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
                Button::new("player-restart-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip(rust_i18n::t!("recorder.restart").to_string())
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
            );

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

/// 将秒数格式化为 `mm:ss` 或 `hh:mm:ss`。
fn format_duration(seconds: f64) -> String {
    let total_secs = seconds.max(0.0).round() as u64;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{mins:02}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}
