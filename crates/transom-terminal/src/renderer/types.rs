//! 终端绘制模型定义：PaintRequest、DamageInfo 与视觉缓存类型。

use super::scan::RunStyle;
use crate::block::BlockLayout;
use crate::event::TerminalEventProxy;
use crate::kitty_graphics::GraphicsPlacement;
use crate::search::SearchPaint;
use alacritty_terminal::term::Term;
use gpui_kit::*;
use std::ops::Range;
use transom_core::BlockViewStyle;

pub(crate) const SELECTION_ALPHA: f32 = 0.4;

/// 一次绘制请求的全部输入。
///
/// 收敛成结构体而不是继续平铺参数：绘制需要网格、IME 预编辑、光标相位与检索命中，
/// 参数已超过 5 个，平铺会让调用点难以对齐语义。
pub(crate) struct PaintRequest<'a> {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) term: &'a Term<TerminalEventProxy>,
    pub(crate) ime: Option<(&'a str, Option<&'a Range<usize>>)>,
    pub(crate) blink_visible: bool,
    /// 检索命中绘制数据；`None` 表示当前没有可用快照（未开启或已失效）。
    pub(crate) search: Option<&'a SearchPaint>,
    /// 语义块折叠后的可见行映射；`None` 表示经典连续流。
    pub(crate) block_layout: Option<&'a BlockLayout>,
    /// 语义块外观样式（卡片/极简）。
    pub(crate) block_style: BlockViewStyle,
    /// 本帧要绘制的 Kitty 图像放置。
    pub(crate) graphics: &'a [GraphicsPlacement],
}

/// 本帧需要重建绘制缓存的行集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DamageInfo {
    /// 全量重建：resize 折行、滚动、INSERT 模式或外观变更导致行号/度量整体失效。
    Full,
    /// 仅重建给出的视口行号（已含 display_offset 换算）。
    Rows(Vec<usize>),
}

/// 一行文本的绘制产物：由 [`TerminalRenderer`] 在脏行上重建，帧间按行复用。
///
/// 设计原理（Why）：
/// - 实现初衷：GPUI 是即时模式绘制，每帧都要完整提交绘制指令；但"从网格扫描
///   出背景区段与文本 run"这一步只依赖行内容，内容不变时重复计算纯属浪费。
///   把扫描产物缓存下来，每帧只做脏行重建与绘制提交；
/// - 核心优势：静止画面（光标闪烁、鼠标移动触发的重绘）不再伴随整屏三遍
///   逐列扫描与 run 重建，文本内容以 SharedString 持有，提交时克隆零分配；
/// - 代价与局限：选区、搜索高亮、IME 与光标绘制走独立覆盖层、不进缓存，
///   因此不受损伤跟踪影响；prepaint 与 paint 之间新到的一帧数据可能以旧
///   内容提交一次（下一帧损伤立即补正），与改造前的错帧窗口同级。
#[derive(Default)]
pub(crate) struct RowVisual {
    /// 非默认背景的区段：(起始列, 结束列, 颜色)。
    pub(crate) background_runs: Vec<(usize, usize, Hsla)>,
    /// 文本 run，按起始列升序。
    pub(crate) text_runs: Vec<TextVisual>,
}

/// 单个文本 run 的缓存形态。
pub(crate) struct TextVisual {
    pub(crate) text: SharedString,
    pub(crate) start_col: usize,
    /// run 覆盖到的列（不含），自绘下划线按列几何取宽度。
    pub(crate) end_col: usize,
    pub(crate) style: RunStyle,
}

/// IME 预编辑绘制输入：位置与网格规模随组字状态成组传递。
pub(crate) struct ImePaintInput<'a> {
    pub(super) cursor_row: i32,
    pub(super) cursor_col: usize,
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) text: &'a str,
    pub(super) marked_range: Option<&'a Range<usize>>,
}
