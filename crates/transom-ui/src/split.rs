//! 标签内分屏：窗格的切分、方向翻转、焦点移动、最大化与摘出为独立标签。

use crate::app::{AppShell, TerminalTab};
use crate::keymap;
use crate::pane_tree::{MAX_PANES, PaneDirection, RemoveOutcome};
use gpui_kit::*;
use transom_core::{Connection, SplitAxis, TabId};

/// 把焦点窗格沿 `axis` 切一半，新窗格复用当前会话配置另开一条连接。
///
/// 只切焦点窗格本身：兄弟窗格及其已拖出的比例原样保留，这是分割树相对
/// 扁平列表的核心收益。串口设备端口独占，不能被第二个窗格重复打开。
pub(crate) fn split_active_pane(
    shell: &mut AppShell,
    axis: SplitAxis,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(active_id) = shell.state.workspace.active_tab else {
        return;
    };
    let Some(group_index) = shell.state.workspace.group_index_for_tab(active_id) else {
        return;
    };
    let Some(session) = shell.state.session_for_tab(active_id) else {
        return;
    };
    if matches!(session.connection, Connection::Serial(_)) {
        shell
            .state
            .set_error(rust_i18n::t!("error.serial_no_split").to_string());
        cx.notify();
        return;
    }
    if shell.state.workspace.groups[group_index].pane_count() >= MAX_PANES {
        shell
            .state
            .set_error(rust_i18n::t!("error.too_many_panes", max = MAX_PANES).to_string());
        cx.notify();
        return;
    }

    let pane_id = TabId::new();
    if !shell.state.workspace.groups[group_index]
        .root
        .split_leaf(active_id, axis, pane_id)
    {
        return;
    }
    let group = &mut shell.state.workspace.groups[group_index];
    group.active_pane = pane_id;
    // 切分后新窗格接管焦点，若仍停留在原窗格的最大化上会把它藏起来。
    group.zoomed = None;
    shell
        .state
        .workspace
        .tabs
        .push(TerminalTab::pending(pane_id, session.clone()));
    shell.state.workspace.active_tab = Some(pane_id);
    crate::connection::start_connection(shell, pane_id, session, cx);
    cx.notify();
}

/// 翻转焦点窗格所在那一刀的方向，外层结构与兄弟窗格尺寸不受影响。
pub(crate) fn toggle_split_axis(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let Some(group_index) = shell.state.active_group_index() else {
        return;
    };
    let active_pane = shell.state.workspace.groups[group_index].active_pane;
    // 单窗格没有父分割节点，toggle_axis 自然返回 false。
    if !shell.state.workspace.groups[group_index]
        .root
        .toggle_axis(active_pane)
    {
        return;
    }
    shell.state.workspace.groups[group_index].zoomed = None;
    cx.notify();
}

/// 在"只看这个窗格"与"按分割树铺满"之间切换。
///
/// 最大化只改渲染视图、不动分割树，因此恢复分屏时布局与比例原样回来。
/// 目标窗格由调用方显式传入，而不是回读"当前活动标签组"：标签栏徽标可以在
/// 未激活的标签上点开，依赖激活顺序会让操作落到错误的组上。
pub(crate) fn toggle_zoom_pane(shell: &mut AppShell, pane: TabId, cx: &mut Context<AppShell>) {
    let Some(group_index) = shell.state.workspace.group_index_for_tab(pane) else {
        return;
    };
    if shell.state.workspace.groups[group_index].pane_count() < 2 {
        return;
    }
    let group = &mut shell.state.workspace.groups[group_index];
    group.zoomed = if group.zoomed == Some(pane) {
        None
    } else {
        Some(pane)
    };
    cx.notify();
}

/// 把窗格摘出为独立标签组：连接、终端视图与滚动缓冲原地保留。
pub(crate) fn detach_pane_to_tab(
    shell: &mut AppShell,
    tab_id: TabId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(group_index) = shell.state.workspace.group_index_for_tab(tab_id) else {
        return;
    };
    if shell.state.workspace.groups[group_index].pane_count() < 2 {
        return;
    }
    let RemoveOutcome::Removed { next_focus } = shell.state.workspace.groups[group_index]
        .root
        .remove_leaf(tab_id)
    else {
        return;
    };
    let group = &mut shell.state.workspace.groups[group_index];
    if group.active_pane == tab_id {
        group.active_pane = next_focus;
    }
    if group.zoomed == Some(tab_id) {
        group.zoomed = None;
    }
    // 摘出后的标签仍属于原会话：SSH 按会话序号继续排；本地终端每次都是
    // 新 Session，改按基础标题序号排，否则会与已有本地标签重名。
    let Some((session_id, base_title, is_local)) = shell.state.workspace.tab(tab_id).map(|tab| {
        (
            tab.session_id,
            tab.session.name.clone(),
            matches!(tab.session.connection, Connection::Local(_)),
        )
    }) else {
        return;
    };
    if is_local {
        shell
            .state
            .workspace
            .create_local_group(tab_id, session_id, &base_title);
    } else {
        shell.state.workspace.create_group(tab_id, session_id);
    }
    shell.state.prune_pane_split_states();
    // 摘出的窗格是一个新标签：单屏显示，原并排对保留；点回配对标签即恢复。
    shell.activate_tab(tab_id, window, cx);
    cx.notify();
}

/// 选中某个窗格：标签栏窗格列表与方向键共用一条规则。
///
/// 放大态下"选中另一个窗格"是换看它，而不是退出放大 —— 否则用户点一次列表
/// 就把自己摆好的最大化视图丢掉了。
pub(crate) fn select_pane(
    shell: &mut AppShell,
    tab_id: TabId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(group_index) = shell.state.workspace.group_index_for_tab(tab_id)
        && shell.state.workspace.groups[group_index].zoomed.is_some()
    {
        shell.state.workspace.groups[group_index].zoomed = Some(tab_id);
    }
    shell.activate_tab(tab_id, window, cx);
}

/// 沿指定方向把焦点移到相邻窗格；该方向已到边时保持不动。
fn focus_neighbor_pane(
    shell: &mut AppShell,
    direction: PaneDirection,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(group_index) = shell.state.active_group_index() else {
        return;
    };
    let active_pane = shell.state.workspace.groups[group_index].active_pane;
    let Some(next) = shell.state.workspace.groups[group_index]
        .root
        .focus_neighbor(active_pane, direction)
    else {
        // 组内该方向已到头：并排视图下再尝试跨到相邻格，单标签视图则保持不动。
        crate::tiling::focus_neighbor_tile(shell, direction, window, cx);
        return;
    };
    crate::split::select_pane(shell, next, window, cx);
}

pub(crate) fn on_split_right(
    shell: &mut AppShell,
    _: &keymap::SplitRight,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::split::split_active_pane(shell, SplitAxis::Horizontal, window, cx);
}

pub(crate) fn on_split_down(
    shell: &mut AppShell,
    _: &keymap::SplitDown,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::split::split_active_pane(shell, SplitAxis::Vertical, window, cx);
}

pub(crate) fn on_close_pane(
    shell: &mut AppShell,
    _: &keymap::ClosePane,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(tab_id) = shell.state.workspace.active_tab {
        shell.close_pane(tab_id, window, cx);
    }
}

pub(crate) fn on_toggle_split_axis(
    shell: &mut AppShell,
    _: &keymap::ToggleSplitAxis,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::split::toggle_split_axis(shell, cx);
}

pub(crate) fn on_zoom_pane(
    shell: &mut AppShell,
    _: &keymap::ZoomPane,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(pane) = shell.state.workspace.active_tab {
        crate::split::toggle_zoom_pane(shell, pane, cx);
    }
}

pub(crate) fn on_focus_pane_left(
    shell: &mut AppShell,
    _: &keymap::FocusPaneLeft,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    focus_neighbor_pane(shell, PaneDirection::Left, window, cx);
}

pub(crate) fn on_focus_pane_right(
    shell: &mut AppShell,
    _: &keymap::FocusPaneRight,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    focus_neighbor_pane(shell, PaneDirection::Right, window, cx);
}

pub(crate) fn on_focus_pane_up(
    shell: &mut AppShell,
    _: &keymap::FocusPaneUp,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    focus_neighbor_pane(shell, PaneDirection::Up, window, cx);
}

pub(crate) fn on_focus_pane_down(
    shell: &mut AppShell,
    _: &keymap::FocusPaneDown,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    focus_neighbor_pane(shell, PaneDirection::Down, window, cx);
}
