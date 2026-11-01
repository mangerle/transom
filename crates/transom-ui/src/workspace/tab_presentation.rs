//! 标签条目的图标与状态色口径。
//!
//! 设计原理（Why）：横向标签栏与纵向标签面板分别计算图标与状态点，纵向
//! 漏掉 ZMODEM 传输色、横向的图标回落顺序也不同；同一标签在两个入口可能
//! 显示不同颜色。把图标/状态色收敛为纯函数，两个入口只消费结果。
//!
//! 代价与局限：只覆盖视觉映射，标签布局仍各按横向/纵向排版。

use crate::app::{AppShell, TerminalTab};
use crate::palette::Palette;
use crate::workspace::pane_status_indicator;
use gpui_kit::assets::IconName;
use gpui_kit::*;

/// 标签图标与图标颜色：播放器/监控看板有专属图标，其余按会话外观回落连接类型。
pub(crate) fn tab_icon(
    shell: &AppShell,
    tab: &TerminalTab,
    active: bool,
    p: &Palette,
) -> (IconName, Hsla) {
    if tab.is_player() {
        return (IconName::Disc, p.muted_foreground);
    }
    if tab.is_monitor() {
        return (IconName::Gauge, p.primary);
    }
    let appearance = shell.state.session_appearance(tab);
    let icon = appearance
        .icon
        .unwrap_or_else(|| crate::connection::icon(&tab.session.connection));
    let color = appearance.color.unwrap_or(if active {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    });
    (icon, color)
}

/// 用户为会话配置的环境色；未配置时不画强调条。
pub(crate) fn tab_accent(shell: &AppShell, tab: &TerminalTab) -> Option<Hsla> {
    shell.state.session_appearance(tab).color
}

/// 标签状态点颜色：ZMODEM 传输中优先用强调色，其余按连接状态。
pub(crate) fn tab_status_dot(tab: &TerminalTab, p: &Palette) -> Hsla {
    if tab.zmodem().is_some() {
        return p.primary;
    }
    pane_status_indicator(&tab.status, p).0
}
