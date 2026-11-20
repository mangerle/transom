//! 标题栏轻量性能 HUD。
//!
//! 设计原理（Why）：这份读数描述的是 Transom 本地窗口，而不是远端主机。帧数据
//! 来自 GPUI profiler 实际记录的绘制帧，资源数据来自 `sysinfo` 对当前进程的
//! 后台采样；渲染期只读缓存，不做进程扫描或文件 IO。
//!
//! 代价与局限：profiler 是进程级功能，多个窗口同时启用 HUD 时需要共享引用计数，
//! 最后一个 HUD 关闭后才能停掉追踪；标题栏空间有限，因此这里刻意不画曲线，只
//! 保留 FPS、帧耗时、CPU 和内存四个关键读数。

mod hud;
mod sampler;

pub(crate) use hud::{performance_hud, set_enabled};

use gpui_kit::*;
use std::collections::HashMap;
use std::time::Duration;

/// 标题栏数字的刷新间隔；与资源采样共用，避免两套计时器互相抢帧。
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
/// FPS 与帧耗时只统计最近一秒，避免旧负载长期拖慢当前读数。
const READOUT_WINDOW: Duration = Duration::from_secs(1);
/// CPU 与内存按最近三秒平均，降低单次系统调用波动造成的读数闪烁。
const RESOURCE_WINDOW: Duration = Duration::from_secs(3);
/// 保留的绘制帧样本上限。
const FRAME_CAPACITY: usize = 120;
/// 首批冷启动帧不计入均值，避免着色器与字体图集初始化污染读数。
const WARMUP_FRAMES: u32 = 8;
/// 连续两个采样周期都没有新 HUD 渲染时，认为标题栏已隐藏并停止后台时钟。
const HIDDEN_TICKS: u8 = 2;

/// 当前窗口的 HUD 实体表。实体随窗口存活，重新开启时沿用同一个实例。
#[derive(Default)]
struct PerformanceHuds(HashMap<WindowId, Entity<hud::PerformanceHud>>);

impl Global for PerformanceHuds {}
