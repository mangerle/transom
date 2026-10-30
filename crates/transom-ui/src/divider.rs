//! 分割线外观：统一可拖拽分割条与区域结构性分隔线的颜色与粗细。
//!
//! # 为什么需要本模块
//! 可拖拽分割条来自 gpui-component 的 `h_resizable`/`v_resizable`，其外观在
//! 依赖内部硬编码：静止是一条 1px 发丝线取主题 `border`，hover 浮出 3px 胶囊
//! 取 `muted_foreground`，应用侧没有配置入口。区域之间的静态分隔线则散落在
//! 各面板，各自写一份 1px 边框。分割线颜色与粗细要能随设置调整，就必须先有
//! 一条唯一的外观来源，否则改了可拖拽条、静态线还是旧色。
//!
//! # 设计取舍
//! - 设置只存语义色（主题边框、弱前景、前景、强调色），渲染时每帧从当前主题
//!   解析：深浅主题切换、窗口透明度折算与主题热重载都自动跟随，不必在换装
//!   回调里重算一份缓存。
//! - 可拖拽条走上游公开的 `with_handle_appearance` 注入自定义渲染，不改依赖
//!   源码；命中热区（跨边 9px、贴边 5px）仍由上游私有常量决定，本模块只接管
//!   "画什么"。
//! - 线宽上限 3px：手柄内容区只有 1px，加宽靠向两侧溢出实现，再宽会明显侵占
//!   相邻面板；静态线用边框实现，过宽还会推动布局。
//!
//! # 覆盖边界
//! 覆盖主窗口的分区边界：活动栏、主内容轮廓、标签栏、窗格标头、状态栏、
//! SFTP 面板与监控抽屉。偏好设置弹窗内的页头线与设置行分割线也取同一档位，
//! 这样改粗细/颜色在弹窗里当场可见。不覆盖其他浮窗与弹窗内部、会话/文件等
//! 列表行分隔、控件内部边框与拖拽落点指示线：它们各有独立语义，跟随全局
//! 分割线只会变成视觉噪音。

use std::rc::Rc;

use gpui_kit::base::{
    ResizablePanelGroup, ResizeHandleContext, ResizeHandleRenderer, ResizeHandleState, Transition,
    h_resizable as base_h_resizable, transition, v_resizable as base_v_resizable,
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use transom_core::{DividerColor, DividerSettings, DividerWidth};

/// 分割线外观在 gpui 全局中的载体。
///
/// 为什么包一层而不是直接实现 `Global`：`DividerSettings` 属于 transom-core，
/// 与 gpui 的 `Global` 特征分属两个 crate，孤儿规则不允许在此为其实现。
#[derive(Clone, Copy, Default)]
struct DividerStyle(DividerSettings);

impl Global for DividerStyle {}

/// 写入分割线外观；由 [`crate::theme::apply_appearance`] 在每次外观变化时调用。
pub(crate) fn apply(cx: &mut App, settings: DividerSettings) {
    cx.set_global(DividerStyle(settings));
}

/// 当前分割线设置；全局尚未写入时按默认档（跟随主题、发丝线）处理，
/// 保证首帧与设置缺失时不出现无样式状态。
pub(crate) fn settings(cx: &App) -> DividerSettings {
    cx.try_global::<DividerStyle>()
        .map(|style| style.0)
        .unwrap_or_default()
}

/// 把语义色解析为当前主题下的具体颜色。
///
/// `Theme` 返回 `None`，表示交给调用点的原有边界色——普通面板边界用
/// `border`，侧栏与 SFTP 面板边界用各自的专用色，默认档因此零观感变化。
pub(crate) fn resolve_color(cx: &App, color: DividerColor) -> Option<Hsla> {
    let theme = cx.theme();
    match color {
        DividerColor::Theme | DividerColor::Unknown => None,
        DividerColor::Border => Some(theme.border),
        DividerColor::Muted => Some(theme.muted_foreground),
        DividerColor::Foreground => Some(theme.foreground),
        DividerColor::Primary => Some(theme.primary),
    }
}

/// 静态分隔线的朝向。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DividerEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// 在容器指定边上加一条分割线边框。
///
/// 为什么做成扩展 trait 而不是普通函数：调用点大多是长链式布局，函数形态
/// 要么被迫把整条链包进括号，要么先落成中间变量再回填，二者都破坏可读性；
/// 方法形态可以直接嵌在链中任意位置，且返回 `Self` 可继续链式追加子元素。
pub(crate) trait DividerStyled: Sized {
    /// 在 `edge` 边上按 `width` 档位加一条分割线边框。
    fn divider_edge(self, edge: DividerEdge, width: DividerWidth) -> Self;
}

impl<T: Styled> DividerStyled for T {
    fn divider_edge(self, edge: DividerEdge, width: DividerWidth) -> Self {
        match edge {
            DividerEdge::Top => match width {
                DividerWidth::Hairline | DividerWidth::Unknown => self.border_t_1(),
                DividerWidth::Medium => self.border_t_2(),
                DividerWidth::Thick => self.border_t_3(),
            },
            DividerEdge::Bottom => match width {
                DividerWidth::Hairline | DividerWidth::Unknown => self.border_b_1(),
                DividerWidth::Medium => self.border_b_2(),
                DividerWidth::Thick => self.border_b_3(),
            },
            DividerEdge::Left => match width {
                DividerWidth::Hairline | DividerWidth::Unknown => self.border_l_1(),
                DividerWidth::Medium => self.border_l_2(),
                DividerWidth::Thick => self.border_l_3(),
            },
            DividerEdge::Right => match width {
                DividerWidth::Hairline | DividerWidth::Unknown => self.border_r_1(),
                DividerWidth::Medium => self.border_r_2(),
                DividerWidth::Thick => self.border_r_3(),
            },
        }
    }
}

/// 水平分割面板组（左右排列，中间是竖向分割条）。
pub(crate) fn h_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    base_h_resizable(id).with_handle_appearance(divider_appearance())
}

/// 垂直分割面板组（上下排列，中间是横向分割条）。
pub(crate) fn v_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    base_v_resizable(id).with_handle_appearance(divider_appearance())
}

/// 本应用的分割条外观渲染器；每次装入手柄时创建一份，渲染期读全局设置。
fn divider_appearance() -> ResizeHandleRenderer {
    Rc::new(|handle, window, cx| Some(render_resize_handle(handle, window, cx)))
}

/// 分割条 hover 后胶囊的长度与不透明度（沿用上游节奏）。
///
/// 静止档不画胶囊：一个停靠区里有十来条分割线，常驻胶囊会变成噪音。
fn indicator(state: ResizeHandleState) -> (Pixels, f32) {
    match state {
        ResizeHandleState::Idle => (px(0.), 0.),
        ResizeHandleState::Hovered => (px(20.), 0.35),
        ResizeHandleState::Pressed => (px(28.), 0.6),
        ResizeHandleState::Dragging => (px(44.), 0.9),
    }
}

/// 画分割条：发丝线加 hover 胶囊，颜色与线宽读当前设置。
fn render_resize_handle(
    handle: &ResizeHandleContext,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let current = settings(cx);
    let width = px(current.width.pixels());
    let line_color = resolve_color(cx, current.color).unwrap_or_else(|| cx.theme().border);
    let pill_color =
        resolve_color(cx, current.color).unwrap_or_else(|| cx.theme().muted_foreground);
    // 胶囊至少比线粗 2px，否则线加粗到 3px 后 hover 反馈被线本身吞掉。
    let pill_thickness = px((current.width.pixels() + 2.0).max(3.0));

    let (target_length, target_opacity) = indicator(handle.state());
    let motion = cx.theme().motion_tokens();
    let policy = Transition::new(motion.duration_fast).easing(motion.easing_move.clone());
    // 两个值每帧都采样：过渡只在运动中请求帧，静止时不产生额外开销；只在
    // 胶囊出现时采样会让下次 hover 从上次残留值开始。
    let length = transition(
        "transom-divider-indicator-length",
        target_length,
        policy.clone(),
        window,
        cx,
    );
    let opacity = transition(
        "transom-divider-indicator-opacity",
        target_opacity,
        policy,
        window,
        cx,
    )
    .clamp(0., 1.);

    div()
        .flex_none()
        .flex()
        .bg(line_color)
        // 加宽后的线只向内容区一侧延伸，不从边界向外溢出：面板容器的
        // overflow 会裁掉越界像素，居中溢出会让贴容器边缘的线只剩一半宽。
        // 代价是线相对边界向内偏 (width - 1) / 2 px，3px 时偏 1px。
        .map(|line| match handle.axis() {
            Axis::Horizontal => line.w(width).h_full().items_center(),
            // 垂直轴沿交叉轴（线厚方向）不能用 items_center：胶囊比线厚，
            // 溢出时对齐不可靠，按上游做法贴起点再手算偏移。
            _ => line.h(width).w_full().items_start().justify_center(),
        })
        .when(length > px(0.5), |line| {
            let pill = div()
                .flex_none()
                .rounded_full()
                .bg(pill_color)
                .opacity(opacity)
                .map(|pill| match handle.axis() {
                    Axis::Horizontal => pill
                        .w(pill_thickness)
                        .h(length)
                        .ml((pill_thickness - width) * -0.5),
                    _ => pill
                        .h(pill_thickness)
                        .w(length)
                        .mt((pill_thickness - width) * -0.5),
                });
            // 贴边手柄的发丝线是容器最外侧一像素，胶囊横跨边界会被停靠区的
            // 裁剪切掉；延迟绘制让它在遮罩之后落笔，保住外侧那半像素。
            line.child(match handle.edge() {
                Some(_) => deferred(pill).into_any_element(),
                None => pill.into_any_element(),
            })
        })
        .into_any_element()
}
