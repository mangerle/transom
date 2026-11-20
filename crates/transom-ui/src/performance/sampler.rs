//! 本地性能 HUD 的帧事件与进程资源采样。

use std::collections::VecDeque;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use gpui_kit::WindowId;
use gpui_kit::profiler::{FrameEvent, FrameTimingCollector, set_trace_enabled};
use parking_lot::Mutex;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};

use super::RESOURCE_WINDOW;

/// 标记当前进程是否由本 HUD 打开了 GPUI profiler。
struct TraceState {
    refs: usize,
    owned_by_host: bool,
}

static TRACE_STATE: LazyLock<Mutex<TraceState>> = LazyLock::new(|| {
    Mutex::new(TraceState {
        refs: 0,
        owned_by_host: false,
    })
});

/// profiler 追踪守护：最后一个 HUD 关闭时恢复原始开关。
pub(super) struct FrameTraceGuard;

impl FrameTraceGuard {
    pub(super) fn acquire() -> Self {
        let mut state = TRACE_STATE.lock();
        if state.refs == 0 {
            // 返回值表示进入前是否已经开启；若宿主自己开了追踪，HUD 退出时
            // 不能替它关掉。
            state.owned_by_host = !set_trace_enabled(true);
        }
        state.refs = state.refs.saturating_add(1);
        Self
    }
}

impl Drop for FrameTraceGuard {
    fn drop(&mut self) {
        let mut state = TRACE_STATE.lock();
        state.refs = state.refs.saturating_sub(1);
        if state.refs == 0 && !state.owned_by_host {
            set_trace_enabled(false);
        }
    }
}

/// 一条实际绘制帧及其发生时间。
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameSample {
    pub(super) at: Instant,
    pub(super) duration: Duration,
}

/// 当前进程的资源读数。
#[derive(Clone, Copy, Debug)]
pub(super) struct ResourceSample {
    pub(super) cpu_percent: f32,
    pub(super) memory_bytes: u64,
}

/// 单窗口帧事件采样器。
pub(super) struct FrameSampler {
    pub(super) collector: FrameTimingCollector,
    pub(super) window_id: WindowId,
    pub(super) frame_samples: VecDeque<FrameSample>,
    pub(super) warmup: u32,
    /// 当前窗口所在显示器的刷新周期；用于把可承载帧率封顶到屏幕实际能力。
    refresh_interval: Option<Duration>,
    own_frame_pending: Option<(Instant, u64)>,
}

impl FrameSampler {
    pub(super) fn new(window_id: WindowId, warmup: u32, capacity: usize) -> Self {
        Self {
            collector: FrameTimingCollector::new(),
            window_id,
            frame_samples: VecDeque::with_capacity(capacity),
            warmup,
            refresh_interval: None,
            own_frame_pending: None,
        }
    }

    /// 收集自上一帧以来的绘制与呈现事件，过滤其他窗口和 HUD 自身触发的帧。
    pub(super) fn collect(&mut self, capacity: usize, readout_window: Duration) {
        for event in self.collector.collect_unseen() {
            match event {
                FrameEvent::Draw(timing) if timing.window_id == self.window_id => {
                    if let Some((since, invalidations)) = self.own_frame_pending
                        && timing.draw_start >= since
                    {
                        self.own_frame_pending = None;
                        if timing.invalidations <= invalidations {
                            continue;
                        }
                    }

                    self.refresh_interval = timing.refresh_interval;
                    if self.warmup > 0 {
                        self.warmup = self.warmup.saturating_sub(1);
                        continue;
                    }
                    if self.frame_samples.len() >= capacity {
                        self.frame_samples.pop_front();
                    }
                    self.frame_samples.push_back(FrameSample {
                        at: timing.draw_end,
                        duration: timing.draw_duration(),
                    });
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let cutoff = now.checked_sub(readout_window).unwrap_or(now);
        while self
            .frame_samples
            .front()
            .is_some_and(|sample| sample.at < cutoff)
        {
            self.frame_samples.pop_front();
        }
    }

    pub(super) fn expect_own_frame(&mut self, at: Instant) {
        self.own_frame_pending = Some(match self.own_frame_pending {
            Some((since, invalidations)) => (since, invalidations.saturating_add(1)),
            None => (at, 1),
        });
    }

    pub(super) fn frame_millis(&self) -> Option<f32> {
        mean_frame_millis(&self.frame_samples)
    }

    /// 按单帧绘制成本折算可承载帧率，并按显示器刷新周期封顶。
    pub(super) fn sustainable_fps(&self) -> Option<f32> {
        let frame_millis = self.frame_millis()?;
        if frame_millis <= 0.0 {
            return None;
        }
        let rate = 1000.0 / frame_millis;
        Some(match self.refresh_interval {
            Some(period) if !period.is_zero() => rate.min(1.0 / period.as_secs_f32()),
            _ => rate,
        })
    }
}

fn mean_frame_millis(samples: &VecDeque<FrameSample>) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let total = samples
        .iter()
        .map(|sample| sample.duration)
        .sum::<Duration>();
    Some(total.as_secs_f32() * 1000.0 / samples.len() as f32)
}

/// 当前进程资源读数的滚动窗口。
struct ResourceHistory {
    samples: VecDeque<(Instant, ResourceSample)>,
    window: Duration,
}

impl ResourceHistory {
    fn new(window: Duration) -> Self {
        Self {
            samples: VecDeque::with_capacity(8),
            window,
        }
    }

    fn push(&mut self, sample: ResourceSample, now: Instant) {
        self.samples.push_back((now, sample));
        while let Some((at, _)) = self.samples.front() {
            if now.duration_since(*at) <= self.window {
                break;
            }
            self.samples.pop_front();
        }
    }

    fn mean(&self) -> Option<ResourceSample> {
        if self.samples.is_empty() {
            return None;
        }
        let count = self.samples.len();
        let cpu_percent = self
            .samples
            .iter()
            .map(|(_, sample)| sample.cpu_percent)
            .sum::<f32>()
            / count as f32;
        let memory_bytes = self
            .samples
            .iter()
            .map(|(_, sample)| u128::from(sample.memory_bytes))
            .sum::<u128>()
            / count as u128;
        Some(ResourceSample {
            cpu_percent,
            memory_bytes: memory_bytes as u64,
        })
    }
}

/// 当前进程的 CPU 与私有内存采样器；只在后台执行器中运行。
pub(super) struct ResourceSampler {
    system: System,
    pid: sysinfo::Pid,
    history: ResourceHistory,
}

impl ResourceSampler {
    pub(super) fn new() -> Option<Self> {
        let pid = get_current_pid().ok()?;
        let mut system = System::new().ok()?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        Some(Self {
            system,
            pid,
            history: ResourceHistory::new(RESOURCE_WINDOW),
        })
    }

    pub(super) fn sample(&mut self) -> Option<ResourceSample> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            false,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let private_memory = private_memory_bytes();
        let process = self.system.process(self.pid)?;
        let sample = ResourceSample {
            cpu_percent: process.cpu_usage(),
            memory_bytes: private_memory.unwrap_or_else(|| process.memory()),
        };
        self.history.push(sample, Instant::now());
        self.history.mean()
    }
}

/// Linux 显示属于进程自身分配的匿名内存，不把共享库和图形栈文件页算进来。
#[cfg(target_os = "linux")]
fn private_memory_bytes() -> Option<u64> {
    const STATUS: &str = "/proc/self/status";
    const ANONYMOUS: &str = "RssAnon:";

    let status = std::fs::read_to_string(STATUS).ok()?;
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix(ANONYMOUS))?;
    let kibibytes: u64 = value.split_whitespace().next()?.parse().ok()?;
    Some(kibibytes.saturating_mul(1024))
}

/// 其他平台暂保留进程常驻内存；后续可按平台私有内存计数器分别实现。
#[cfg(not(target_os = "linux"))]
fn private_memory_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::{
        FrameSample, FrameSampler, ResourceHistory, ResourceSample, mean_frame_millis,
        private_memory_bytes,
    };
    use gpui_kit::WindowId;
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    #[test]
    fn frame_mean_is_reported_in_milliseconds() {
        let mut samples = VecDeque::new();
        samples.push_back(FrameSample {
            at: Instant::now(),
            duration: Duration::from_millis(4),
        });
        samples.push_back(FrameSample {
            at: Instant::now(),
            duration: Duration::from_millis(8),
        });
        assert!(mean_frame_millis(&samples).is_some_and(|value| (value - 6.0).abs() < 0.01));
    }

    #[test]
    fn sustainable_fps_is_derived_from_draw_cost_and_capped_by_display() {
        let mut sampler = FrameSampler::new(WindowId::from(1), 0, 8);
        sampler.frame_samples.push_back(FrameSample {
            at: Instant::now(),
            duration: Duration::from_millis(6),
        });
        sampler.refresh_interval = Some(Duration::from_nanos(16_666_667));
        assert!(
            sampler
                .sustainable_fps()
                .is_some_and(|value| (value - 60.0).abs() < 0.01)
        );

        sampler.refresh_interval = None;
        assert!(
            sampler
                .sustainable_fps()
                .is_some_and(|value| (value - 166.67).abs() < 0.1)
        );
    }

    #[test]
    fn resource_history_averages_within_the_window() {
        let now = Instant::now();
        let mut history = ResourceHistory::new(Duration::from_secs(3));
        history.push(
            ResourceSample {
                cpu_percent: 2.0,
                memory_bytes: 60,
            },
            now,
        );
        history.push(
            ResourceSample {
                cpu_percent: 8.0,
                memory_bytes: 80,
            },
            now + Duration::from_millis(500),
        );
        let sample = history.mean();
        assert!(sample.is_some_and(|sample| {
            (sample.cpu_percent - 5.0).abs() < 0.01 && sample.memory_bytes == 70
        }));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn anonymous_memory_is_a_part_of_the_resident_set() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let resident = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .map(|kibibytes| kibibytes.saturating_mul(1024));
        let anonymous = private_memory_bytes();
        if let (Some(resident), Some(anonymous)) = (resident, anonymous) {
            assert!(anonymous <= resident);
        }
    }
}
