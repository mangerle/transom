//! 监控面板抽屉宽度与独立标签页最大化/停靠切换。

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::app::{AppShell, TabContent, TerminalTab};
use gpui_kit::{Context, Window};
use transom_core::TabId;

/// 面板已改好宽度后，外壳侧同步持久化并让分栏重新分配尺寸。
///
/// 宽度状态由面板写入（见 `MonitorView::set_drawer_width`）；面板点击回调
/// 执行期间实体被租借，这里只做设置落盘与 `ResizableState` 调整。
pub(crate) fn apply_monitor_drawer_width(
    shell: &mut AppShell,
    new_width: f32,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    update_app_settings(cx, |s| s.monitor.drawer_width = new_width);
    shell.persist_layout_settings(cx);
    let panels_state = if app_settings(cx).window.sftp_panel_side.is_left() {
        &shell.state.workspace_panels_left
    } else {
        &shell.state.workspace_panels
    };
    let panel_count = panels_state.read(cx).sizes().len();
    if panel_count > 0 {
        let monitor_ix = panel_count - 1;
        panels_state.update(cx, |state, cx| {
            state.resize_panel(monitor_ix, gpui_kit::px(new_width), window, cx);
        });
    }
    cx.notify();
}

/// 将监控抽屉脱离为全屏独立的 Workspace Tab。
pub(crate) fn maximize_monitor_to_tab(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    // 1. 寻找当前关联的主机会话 TabId
    let active_tab_id = shell.state.workspace.active_tab;
    let target_tab_id = active_tab_id
        .and_then(|id| shell.state.workspace.tab(id))
        .and_then(|tab| {
            if let TabContent::Monitor(tid) = tab.content {
                Some(tid)
            } else if !tab.is_player() {
                Some(tab.id)
            } else {
                None
            }
        })
        .or_else(|| {
            shell
                .state
                .workspace
                .tabs
                .iter()
                .find(|tab| !tab.is_player() && !tab.is_monitor())
                .map(|tab| tab.id)
        });

    let Some(target_tab_id) = target_tab_id else {
        return;
    };

    // 2. 如果已经存在针对该 target_tab_id 的监控 Tab，直接切过去
    if let Some(existing) = shell.state.workspace.tabs.iter().find(|t| {
        if let TabContent::Monitor(tid) = t.content {
            tid == target_tab_id
        } else {
            false
        }
    }) {
        let existing_id = existing.id;
        shell.activate_tab(existing_id, window, cx);
        cx.notify();
        return;
    }

    // 3. 构建全屏监控独立标签页
    let Some(target_tab) = shell.state.workspace.tab(target_tab_id) else {
        return;
    };
    let target_title = target_tab.display_name();
    let target_session = target_tab.session.clone();
    let target_connection_id = target_tab.connection_id;
    let target_session_id = target_tab.session_id;

    let monitor_tab_id = TabId::new();
    let tab_title = rust_i18n::t!("monitor.view.tab_name", host = &target_title).to_string();

    let new_tab = TerminalTab::monitor_tab(
        monitor_tab_id,
        target_session,
        tab_title,
        target_tab_id,
        target_connection_id,
    );

    shell
        .state
        .workspace
        .create_group(monitor_tab_id, target_session_id);
    shell.state.workspace.tabs.push(new_tab);
    shell.activate_tab(monitor_tab_id, window, cx);
    cx.notify();
}

/// 将全屏监控独立标签页收回并停靠为右侧抽屉。
pub(crate) fn dock_monitor_to_drawer(
    shell: &mut AppShell,
    tab_id: TabId,
    target_tab_id: TabId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    // 抽屉的开合由面板在调用前写好；这里只做标签切换与关闭。
    // 2. 切回原会话标签页
    if shell.state.workspace.tab(target_tab_id).is_some() {
        shell.activate_tab(target_tab_id, window, cx);
    }

    // 3. 关闭全屏监控 Tab
    shell.close_pane(tab_id, window, cx);
    cx.notify();
}
