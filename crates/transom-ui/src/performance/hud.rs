//! 标题栏 HUD 的实体状态、渲染与调度。

use std::time::{Duration, Instant};

use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use super::sampler::{FrameSampler, FrameTraceGuard, ResourceSample, ResourceSampler};
use super::{FRAME_CAPACITY, HIDDEN_TICKS, READOUT_WINDOW, SAMPLE_INTERVAL, WARMUP_FRAMES};
use crate::format::format_compact_size;
use crate::palette::Palette;
use crate::theme::tokens::{control, space, typography};

/// 60Hz 帧预算，用于帧耗时语义着色。
const FRAME_BUDGET: Duration = Duration::from_nanos(16_666_667);
/// FPS 数值槽：容纳四位数并右对齐，避免位数变化推挤整条 HUD。
const FPS_VALUE_WIDTH: f32 = 2.0;
/// 帧耗时数值槽：容纳 `123.4 ms`。
const FRAME_VALUE_WIDTH: f32 = 3.75;
/// CPU 数值槽：容纳 `100.0%`。
const CPU_VALUE_WIDTH: f32 = 3.25;
/// 内存数值槽：容纳 `1023 MB` 与 `1.2 GB`。
const MEMORY_VALUE_WIDTH: f32 = 3.75;

#[derive(Clone, Copy, Debug, Default)]
struct PerformanceReadout {
    sustainable_fps: Option<f32>,
    frame_millis: Option<f32>,
}

/// 单个 HUD 指标行的渲染输入。
///
/// 数值宽度与标签一起传入，避免渲染函数用字符串长度反推布局；固定槽位是
/// 防止 500ms 刷新时整条标题栏左右抖动的前提。
struct MetricRender<'a> {
    label: &'a str,
    value: String,
    value_width: f32,
    value_color: Hsla,
    palette: &'a Palette,
}

/// 获取当前窗口的 HUD 实体；第一次启用时创建。
pub(crate) fn performance_hud(
    window: &mut Window,
    enabled: bool,
    cx: &mut App,
) -> Entity<PerformanceHud> {
    let window_id = window.window_handle().window_id();
    let existing = cx
        .try_global::<super::PerformanceHuds>()
        .and_then(|huds| huds.0.get(&window_id).cloned());
    if let Some(hud) = existing {
        if enabled {
            hud.update(cx, |hud, cx| hud.set_enabled(true, cx));
        }
        return hud;
    }

    let hud = cx.new(|cx| PerformanceHud::new(window_id, enabled, cx));
    cx.default_global::<super::PerformanceHuds>()
        .0
        .insert(window_id, hud.clone());
    hud
}

/// 设置变化后同步所有已创建 HUD；未创建的实体会在下一次标题栏渲染时按新设置创建。
pub(crate) fn set_enabled(enabled: bool, cx: &mut App) {
    let huds: Vec<Entity<PerformanceHud>> = cx
        .try_global::<super::PerformanceHuds>()
        .map(|huds| huds.0.values().cloned().collect())
        .unwrap_or_default();
    for hud in huds {
        hud.update(cx, |hud, cx| hud.set_enabled(enabled, cx));
    }
}

/// 一个窗口的标题栏性能监视器。
pub(crate) struct PerformanceHud {
    enabled: bool,
    trace_guard: Option<FrameTraceGuard>,
    sampler: FrameSampler,
    readout: PerformanceReadout,
    resources: Option<ResourceSample>,
    clock: Option<Task<()>>,
    clock_running: bool,
    frames_rendered: u64,
}

impl PerformanceHud {
    pub(super) fn new(window_id: WindowId, enabled: bool, _cx: &mut Context<Self>) -> Self {
        let trace_guard = enabled.then(FrameTraceGuard::acquire);
        Self {
            enabled,
            trace_guard,
            sampler: FrameSampler::new(window_id, WARMUP_FRAMES, FRAME_CAPACITY),
            readout: PerformanceReadout::default(),
            resources: None,
            clock: None,
            clock_running: false,
            frames_rendered: 0,
        }
    }

    /// 同步持久化开关；关闭时立即停止后台采样并释放 profiler 引用。
    fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.clock = None;
        self.clock_running = false;
        if enabled {
            self.trace_guard = Some(FrameTraceGuard::acquire());
            self.reset();
            self.start_clock(cx);
        } else {
            self.trace_guard = None;
            self.reset();
        }
        cx.notify();
    }

    fn reset(&mut self) {
        let window_id = self.sampler.window_id;
        self.sampler = FrameSampler::new(window_id, WARMUP_FRAMES, FRAME_CAPACITY);
        self.readout = PerformanceReadout::default();
        self.resources = None;
    }

    /// 后台时钟负责每 500ms 刷新数字和采样当前进程资源。
    fn start_clock(&mut self, cx: &mut Context<Self>) {
        if !self.enabled || self.clock_running {
            return;
        }
        self.clock_running = true;
        let initial_frames = self.frames_rendered;
        self.clock = Some(cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            // 进程扫描与 CPU 差分都是阻塞式系统调用，只允许在后台执行器里运行。
            let mut probe = executor.spawn(async { ResourceSampler::new() }).await;
            let mut watch = RenderWatch::new(initial_frames, HIDDEN_TICKS);
            loop {
                executor.timer(SAMPLE_INTERVAL).await;
                let sample = match probe.take() {
                    Some(mut sampler) => {
                        let (returned, sample) = executor
                            .spawn(async move {
                                let sample = sampler.sample();
                                (sampler, sample)
                            })
                            .await;
                        probe = Some(returned);
                        sample
                    }
                    None => None,
                };

                let keep_running = this.update(cx, |this, cx| {
                    if !this.enabled || watch.is_hidden(this.frames_rendered) {
                        this.clock_running = false;
                        return false;
                    }
                    if sample.is_some() {
                        this.resources = sample;
                    }
                    this.refresh_readout();
                    // 下一次重绘由 HUD 自己触发，不把它计入应用帧耗时。
                    this.sampler.expect_own_frame(Instant::now());
                    cx.notify();
                    true
                });
                if !matches!(keep_running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn refresh_readout(&mut self) {
        self.sampler.collect(FRAME_CAPACITY, READOUT_WINDOW);
        self.readout = PerformanceReadout {
            sustainable_fps: self.sampler.sustainable_fps(),
            frame_millis: self.sampler.frame_millis(),
        };
    }

    fn render_metric(&self, input: MetricRender<'_>) -> AnyElement {
        let MetricRender {
            label,
            value,
            value_width,
            value_color,
            palette,
        } = input;
        h_flex()
            .gap(rems(space::XS))
            .items_center()
            .flex_none()
            .child(
                div()
                    .flex_none()
                    .text_color(palette.muted_foreground)
                    .font_weight(FontWeight::MEDIUM)
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .w(rems(value_width))
                    .text_right()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(value_color)
                    .child(value),
            )
            .into_any_element()
    }

    fn divider(p: &Palette) -> AnyElement {
        div()
            .h(rems(space::MD))
            .w(px(1.0))
            .bg(p.border.opacity(0.55))
            .into_any_element()
    }
}

impl Render for PerformanceHud {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames_rendered = self.frames_rendered.saturating_add(1);
        self.refresh_readout();
        self.start_clock(cx);

        let p = Palette::from(cx);
        let fps = self.readout.sustainable_fps;
        let cpu = self
            .resources
            .map(|sample| format_cpu(sample.cpu_percent))
            .unwrap_or_else(|| "--".to_string());
        let memory = self
            .resources
            .map(|sample| format_compact_size(sample.memory_bytes))
            .unwrap_or_else(|| "--".to_string());
        let fps_color = fps.map_or(p.muted_foreground, |value| fps_color(value, &p));
        let frame_color = self
            .readout
            .frame_millis
            .map_or(p.muted_foreground, |value| frame_time_color(value, &p));

        h_flex()
            .id("titlebar-performance-hud")
            .flex_shrink_1()
            .h(rems(control::ICON_BUTTON))
            .min_w_0()
            .gap(rems(space::SM))
            .items_center()
            .px(rems(space::SM))
            .rounded(p.radius.sm)
            .border_1()
            .border_color(p.border.opacity(0.55))
            .bg(p.surface.active)
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(rems(typography::CAPTION))
            .child(self.render_metric(MetricRender {
                label: "FPS",
                value: fps.map_or_else(|| "--".to_string(), |value| format!("{value:.0}")),
                value_width: FPS_VALUE_WIDTH,
                value_color: fps_color,
                palette: &p,
            }))
            .child(Self::divider(&p))
            .child(
                self.render_metric(MetricRender {
                    label: &rust_i18n::t!("performance.frame"),
                    value: self
                        .readout
                        .frame_millis
                        .map_or_else(|| "--".to_string(), |value| format!("{value:.1} ms")),
                    value_width: FRAME_VALUE_WIDTH,
                    value_color: frame_color,
                    palette: &p,
                }),
            )
            .child(Self::divider(&p))
            .child(self.render_metric(MetricRender {
                label: "CPU",
                value: cpu,
                value_width: CPU_VALUE_WIDTH,
                value_color: p.foreground,
                palette: &p,
            }))
            .child(Self::divider(&p))
            .child(self.render_metric(MetricRender {
                label: &rust_i18n::t!("performance.memory"),
                value: memory,
                value_width: MEMORY_VALUE_WIDTH,
                value_color: p.foreground,
                palette: &p,
            }))
    }
}

impl Drop for PerformanceHud {
    fn drop(&mut self) {
        self.trace_guard = None;
    }
}

fn fps_color(fps: f32, p: &Palette) -> Hsla {
    if fps >= 55.0 {
        p.success
    } else if fps >= 30.0 {
        p.warning
    } else {
        p.danger
    }
}

fn frame_time_color(millis: f32, p: &Palette) -> Hsla {
    if millis <= FRAME_BUDGET.as_secs_f32() * 1000.0 {
        p.success
    } else if millis <= FRAME_BUDGET.as_secs_f32() * 2000.0 {
        p.warning
    } else {
        p.danger
    }
}

fn format_cpu(percent: f32) -> String {
    if percent < 10.0 {
        format!("{percent:.2}%")
    } else {
        format!("{percent:.1}%")
    }
}

/// 连续多个时钟周期看不到 HUD 重绘时停止后台时钟。
struct RenderWatch {
    last_frames: u64,
    unchanged_ticks: u8,
    hidden_ticks: u8,
}

impl RenderWatch {
    fn new(last_frames: u64, hidden_ticks: u8) -> Self {
        Self {
            last_frames,
            unchanged_ticks: 0,
            hidden_ticks,
        }
    }

    fn is_hidden(&mut self, frames: u64) -> bool {
        if frames == self.last_frames {
            self.unchanged_ticks = self.unchanged_ticks.saturating_add(1);
        } else {
            self.last_frames = frames;
            self.unchanged_ticks = 0;
        }
        self.unchanged_ticks >= self.hidden_ticks
    }
}

#[cfg(test)]
mod tests {
    use super::RenderWatch;

    #[test]
    fn hidden_after_the_configured_number_of_unchanged_ticks() {
        let mut watch = RenderWatch::new(3, 2);
        assert!(!watch.is_hidden(3));
        assert!(watch.is_hidden(3));
        assert!(!watch.is_hidden(4));
    }
}
