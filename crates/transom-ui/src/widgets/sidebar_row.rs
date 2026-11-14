//! 侧栏列表框架、普通行与分组行的共享视觉外壳。
//!
//! 设计原理（Why）：会话、容器与 Git 侧栏是同一位置的同类导航清单，行高、内边距、
//! 悬浮、选中和当前项必须由一份实现决定。页面只提供图标、文本与尾部控件，避免三处
//! 各自维护颜色和边框后再次漂移。
//!
//! 代价与局限：组件不接管点击、拖拽和右键菜单，这些交互仍由各领域调用方追加；因此
//! 它只保证视觉状态一致，不限制业务动作。

use crate::palette::Palette;
use crate::theme::tokens::{size, space};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;

/// 普通侧栏行的三态。
///
/// `Current` 始终优先于 `Selected`，由调用方在构造状态时完成优先级判定；枚举本身
/// 不携带业务数据，使表现层无需理解会话、引擎或 Git 项目。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidebarRowState {
    /// 常态行：底色透明，悬浮时显示列表悬浮色。
    Normal,
    /// 已选中但非当前打开项：列表激活色加弱强调边界。
    Selected,
    /// 当前打开项：列表激活色加左侧强调竖条。
    Current,
}

impl SidebarRowState {
    /// 是否应使用强调前景色及行内高亮控件的颜色。
    pub(crate) fn is_highlighted(self) -> bool {
        matches!(self, Self::Selected | Self::Current)
    }
}

/// 侧栏列表的共享留白容器。
///
/// 三个侧栏统一使用同一内边距；滚动与显隐仍由调用方决定，因为会话侧栏使用虚拟列表，
/// 容器与 Git 侧栏使用普通滚动区域。
pub(crate) fn sidebar_list_frame(id: impl Into<ElementId>) -> Stateful<Div> {
    v_flex()
        .id(id.into())
        .w_full()
        .min_h_0()
        .p(rems(space::SM))
        .gap(px(2.0))
}

/// 构造普通侧栏行的视觉外壳。
///
/// 返回的元素已经包含行高、内边距、间距、圆角、文字色、悬浮态、选中边界与当前项
/// 左竖条；调用方继续追加图标、两行文字、尾部状态和拖放遮罩。
pub(crate) fn sidebar_row_band(
    id: impl Into<ElementId>,
    state: SidebarRowState,
    p: &Palette,
) -> Stateful<Div> {
    let foreground = if state.is_highlighted() {
        p.sidebar_accent_foreground
    } else {
        p.sidebar_foreground
    };
    let current_bg = p.surface.control_selected;
    let subtle_border = p.stroke.subtle;
    let hover_bg = p.list_hover;
    let mut band = h_flex()
        .id(id.into())
        .relative()
        .w_full()
        .h(rems(size::TREE_BAND_HEIGHT))
        .px(rems(space::SM))
        .gap(rems(space::SM))
        .items_center()
        .rounded(p.radius.md)
        .text_color(foreground);

    match state {
        SidebarRowState::Normal => {
            band = band.hover(move |style| style.bg(hover_bg));
        }
        SidebarRowState::Selected => {
            band = band.bg(current_bg).child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(subtle_border),
            );
        }
        SidebarRowState::Current => {
            // 纯净中性灰胶囊选中态：去掉左侧蓝色竖条，与活动栏统一。
            band = band.bg(current_bg);
        }
    }
    band
}

/// 构造侧栏分组行的视觉外壳。
///
/// 分组行不参与单选，因此只统一组头底色、底部分隔线、圆角和悬浮反馈；折叠、拖放及
/// 菜单仍由调用方追加。
pub(crate) fn sidebar_group_row_band(id: impl Into<ElementId>, p: &Palette) -> Stateful<Div> {
    let hover_bg = p.list_hover;
    h_flex()
        .id(id.into())
        .relative()
        .w_full()
        .h(rems(size::TREE_BAND_HEIGHT))
        .px(rems(space::SM))
        .gap(rems(space::XS))
        .items_center()
        .rounded(p.radius.md)
        .bg(p.list_head)
        .text_color(p.sidebar_foreground)
        .hover(move |style| style.bg(hover_bg))
}
