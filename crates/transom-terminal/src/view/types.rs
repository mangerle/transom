//! 终端视图布局度量与预备帧类型定义。

use super::silent_echo::SilentEcho;
use crate::block::BlockLayout;
use crate::kitty_graphics::GraphicsPlacement;
use gpui_kit::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 网格尺寸变化的去抖窗口：尺寸在此窗口内稳定下来才真正下发。
///
/// 设计原理（Why）：拖拽窗口边框、展开或收起侧边面板都会连着产生十几个中间尺寸。
/// 每个尺寸都下发一次的话，远端程序会在旧宽度下继续输出，而本地网格已按新宽度重排，
/// 这些行被硬切时写下的软换行标记（WRAPLINE）就此落在错误的列上——之后拉宽只认这个
/// 标记合并行，原始边界已经丢失，历史内容无法复原，且每开合一次都在坏数据上再坏一次。
pub(crate) const RESIZE_DEBOUNCE: Duration = Duration::from_millis(80);

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
    /// 失败命令（非 0 退出码）在刻度带上的位置；检索开启时为空，两者不共用刻度语义。
    pub(crate) failure_ticks: Vec<f32>,
    /// 本帧需要重建行缓存的损伤集合。
    pub(crate) damage: crate::renderer::DamageInfo,
    /// 本帧的语义块折叠映射；`None` 表示经典连续流。
    pub(crate) block_layout: Option<Arc<BlockLayout>>,
    /// 本帧要绘制的 Kitty 图像放置。
    pub(crate) graphics: Vec<GraphicsPlacement>,
}

/// 待下发的网格尺寸与去抖截止时刻。
pub(crate) struct PendingResize {
    pub(crate) size: transom_core::TerminalSize,
    pub(crate) deadline: Instant,
}

/// 尺寸下发的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeDecision {
    /// 目标尺寸与当前生效尺寸一致，无需任何动作。
    Unchanged,
    /// 出现新的目标尺寸：重新计时，等待尺寸稳定下来。
    Restart,
    /// 目标尺寸正在计时中，尚未到下发时刻。
    Wait,
    /// 已到下发时刻，可以真正下发。
    Ready,
}

/// 判定目标尺寸是否可以真正下发。
///
/// 判定与副作用分离，便于单测把去抖语义锁死：只有目标尺寸连续稳定满
/// `RESIZE_DEBOUNCE` 才会下发，途中换成别的尺寸就重新计时。
pub(crate) fn decide_resize(
    current: transom_core::TerminalSize,
    target: transom_core::TerminalSize,
    pending: Option<&PendingResize>,
    now: Instant,
) -> ResizeDecision {
    if current == target {
        return ResizeDecision::Unchanged;
    }
    match pending {
        Some(item) if item.size == target => {
            if now >= item.deadline {
                ResizeDecision::Ready
            } else {
                ResizeDecision::Wait
            }
        }
        _ => ResizeDecision::Restart,
    }
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
    /// 注入命令回显过滤器：读取线程据此把 Shell 回显的整段脚本摘掉。
    pub(crate) silent_echo: parking_lot::Mutex<SilentEcho>,
    /// 本帧终端画布的命中盒：悬停超链接时用它把系统光标切成手型。
    ///
    /// GPUI 的光标样式必须挂在具体 hitbox 上，且 hitbox 只在预备帧生成；
    /// 这里保存最近一帧的句柄，鼠标事件里直接复用，避免为了改光标再建一个元素。
    pub(crate) canvas_hitbox: parking_lot::Mutex<Option<gpui_kit::Hitbox>>,
    /// 待下发的网格尺寸；`None` 表示当前没有等待下发的尺寸变化。
    ///
    /// 尺寸变化先落在这里，等它稳定 `RESIZE_DEBOUNCE` 之后才真正下发，
    /// 免得拖拽与面板动画产生的中间尺寸逐个把远端输出的换行标记写坏。
    pub(crate) pending_resize: parking_lot::Mutex<Option<PendingResize>>,
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
            silent_echo: parking_lot::Mutex::new(SilentEcho::default()),
            canvas_hitbox: parking_lot::Mutex::new(None),
            pending_resize: parking_lot::Mutex::new(None),
        }
    }
}
