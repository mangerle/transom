//! 终端视图布局度量与预备帧类型定义。

use crate::block::BlockLayout;
use crate::kitty_graphics::GraphicsPlacement;
use gpui_kit::*;
use std::sync::Arc;

pub const CONTEXT: &str = "Terminal";

/// 检索刻度带的分桶高度（像素）：命中再多也按像素合并，避免一帧画出上千个矩形。
pub(crate) const SEARCH_TICK_PIXELS: f32 = 2.0;

/// 终端读取线程单次读取的字节数。
pub(crate) const READ_BUFFER_BYTES: usize = 64 * 1024;

/// 读取块通道深度：64 KiB × 32 ≈ 2 MiB 上限，与原 8 KiB × 128 同量级。
pub(crate) const READ_CHANNEL_CAPACITY: usize = 32;

pub type KeyFilter = Arc<dyn Fn(&Keystroke) -> bool + Send + Sync>;

#[derive(Clone, Copy, Default)]
pub(crate) struct LayoutMetrics {
    pub(crate) origin: Point<Pixels>,
    pub(crate) cell_width: Pixels,
    pub(crate) cell_height: Pixels,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    /// 检索刻度带的矩形；仅在检索栏打开的一帧里有值，供鼠标跳转命中测试。
    pub(crate) search_strip: Option<Bounds<Pixels>>,
    /// 终端画布在本帧占据的矩形；悬浮操作胶囊按它定位。
    pub(crate) bounds: Option<Bounds<Pixels>>,
}

pub(crate) struct PreparedTerminal {
    /// 本帧可用的检索绘制数据；`None` 表示无检索或快照已失效。
    pub(crate) search: Option<crate::search::SearchPaint>,
    /// 本帧需要重建行缓存的损伤集合。
    pub(crate) damage: crate::renderer::DamageInfo,
    /// 本帧的语义块折叠映射；`None` 表示经典连续流。
    pub(crate) block_layout: Option<Arc<BlockLayout>>,
    /// 本帧要绘制的 Kitty 图像放置。
    pub(crate) graphics: Vec<GraphicsPlacement>,
}

/// 终端视图共享的内部渲染与引擎状态容器。
///
/// 将原本散落在 `TerminalView` 各处的多个独立 `Arc<Mutex<...>>`、`Arc<RwLock<...>>`、
/// `Arc<AtomicBool>` 内聚收敛至单一复合状态对象中。
/// 既消除了绘制准备与提交阶段在闭包中一次性 clone 10+ 散乱智能指针的“闭包克隆风暴”，
/// 又统一了终端引擎与渲染管线之间的数据归属与状态生命周期。
pub(crate) struct TerminalRenderState {
    pub(crate) engine: parking_lot::Mutex<crate::engine::TerminalEngine>,
    pub(crate) metrics: parking_lot::Mutex<LayoutMetrics>,
    pub(crate) renderer: parking_lot::RwLock<crate::renderer::TerminalRenderer>,
    pub(crate) renderer_dirty: std::sync::atomic::AtomicBool,
    pub(crate) row_cache: parking_lot::Mutex<Vec<crate::renderer::RowVisual>>,
    pub(crate) block_layout: parking_lot::Mutex<Option<Arc<BlockLayout>>>,
    pub(crate) trigger_engine: parking_lot::Mutex<crate::trigger::TriggerEngine>,
}

impl TerminalRenderState {
    pub(crate) fn new(
        engine: crate::engine::TerminalEngine,
        renderer: crate::renderer::TerminalRenderer,
    ) -> Self {
        Self {
            engine: parking_lot::Mutex::new(engine),
            metrics: parking_lot::Mutex::new(LayoutMetrics::default()),
            renderer: parking_lot::RwLock::new(renderer),
            renderer_dirty: std::sync::atomic::AtomicBool::new(false),
            row_cache: parking_lot::Mutex::new(Vec::new()),
            block_layout: parking_lot::Mutex::new(None),
            trigger_engine: parking_lot::Mutex::new(crate::trigger::TriggerEngine::default()),
        }
    }
}
