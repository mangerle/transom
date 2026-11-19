//! 监控面板抽屉宽度与独立标签页最大化/停靠切换。

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::app::{AppShell, AppState, WorkspaceTab};
use gpui_kit::{Context, Window};
use transom_core::TabId;

/// 面板已改好宽度后，外壳侧同步持久化并让分栏重新分配尺寸。
///
/// 宽度状态由面板写入（见 `MonitorView::set_drawer_width`）；面板点击回调
/// 执行期间实体被租借，这里只做设置落盘与 `ResizableState` 调整。
pub(crate) fn apply_monitor_drawer_width(
    app_state: &mut AppState,
    new_width: f32,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    update_app_settings(cx, |s| s.monitor.drawer_width = new_width);
    app_state.persist_layout_settings(cx);
    let panels_state = if app_settings(cx).window.sftp_panel_side.is_left() {
        &app_state.ui.workspace_panels_left
    } else {
        &app_state.ui.workspace_panels
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
    app_state: &mut AppState,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    // 1. 寻找当前关联的主机会话 TabId
    let active_tab_id = app_state.workspace.active_tab;
    let target_tab_id = active_tab_id
        .and_then(|id| app_state.workspace.tab(id))
        .and_then(|tab| {
            if let Some(monitor) = tab.monitor() {
                Some(monitor.target_tab_id)
            } else if tab.is_session() {
                Some(tab.id)
            } else {
                None
            }
        })
        .or_else(|| {
            app_state
                .workspace
                .tabs
                .iter()
                .find(|tab| tab.is_session())
                .map(|tab| tab.id)
        });

    let Some(target_tab_id) = target_tab_id else {
        return;
    };

    // 2. 如果已经存在针对该 target_tab_id 的监控 Tab，直接切过去
    if let Some(existing) = app_state.workspace.tabs.iter().find(|tab| {
        tab.monitor()
            .is_some_and(|monitor| monitor.target_tab_id == target_tab_id)
    }) {
        let existing_id = existing.id;
        app_state.ui.window_requests.push_activate_tab(existing_id);
        cx.notify();
        return;
    }

    // 3. 构建全屏监控独立标签页
    let Some(target_tab) = app_state.workspace.tab(target_tab_id) else {
        return;
    };
    let target_title = target_tab.display_name();
    let Some(target_session) = target_tab.session().cloned() else {
        return;
    };

    let monitor_tab_id = TabId::new();
    let tab_title = rust_i18n::t!("monitor.view.tab_name", host = &target_title).to_string();

    let new_tab =
        WorkspaceTab::new_monitor(monitor_tab_id, tab_title, target_tab_id, target_session);

    app_state.workspace.create_group_for_tab(&new_tab);
    app_state.workspace.tabs.push(new_tab);
    app_state
        .ui
        .window_requests
        .push_activate_tab(monitor_tab_id);
    cx.notify();
}

/// 将全屏监控独立标签页收回并停靠为右侧抽屉。
pub(crate) fn dock_monitor_to_drawer(
    app_state: &mut AppState,
    tab_id: TabId,
    target_tab_id: TabId,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    // 抽屉的开合由面板在调用前写好；这里只做标签切换与关闭。
    // 2. 切回原会话标签页
    if app_state.workspace.tab(target_tab_id).is_some() {
        app_state
            .ui
            .window_requests
            .push_activate_tab(target_tab_id);
    }

    // 3. 关闭全屏监控 Tab
    app_state.ui.window_requests.push_close_pane(tab_id);
    cx.notify();
}
