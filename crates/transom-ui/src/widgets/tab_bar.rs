//! 统一的下划线标签栏原语：共享底线轨道与激活态蓝线覆盖。
//!
//! # 设计原理（Why）
//! - 此前部分模块使用外层 `border_b_1()` 叠加内层 `border_b_2()`，在盒模型中会形成
//!   “1px 灰线之上再叠加 2px 蓝线”的双层线，且边框会向上挤压标签文字；
//! - 统一采用“容器底部铺设绝对定位底线 + 激活标签底部绝对定位强调线直接覆盖”模型，
//!   保证全应用所有下划线标签的蓝线位置、粗细与视觉表现完全一致。

use crate::palette::Palette;
use gpui_kit::*;

/// 标签栏底部的全局基准灰线（绝对定位铺满底边，宽度随 `p.divider_width` 变化）。
pub(crate) fn underline_tab_bar_line(p: &Palette) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(p.divider_width.pixels()))
        .bg(p.divider(p.border))
}

/// 激活标签底部的蓝色强调线（绝对定位覆盖在基准灰线同一条基线上，高度不低于 2px）。
pub(crate) fn underline_tab_active_line(p: &Palette) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(p.divider_width.pixels().max(2.0)))
        .bg(p.primary)
}
