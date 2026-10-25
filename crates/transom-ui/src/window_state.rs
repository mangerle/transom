//! 窗口几何与标签组状态的持久化。

use crate::app::{AppShell, TerminalTab};
use gpui_kit::*;
use transom_core::{Connection, SavedTabGroup};

impl AppShell {
    pub(crate) fn save_window_state(&mut self, window: &mut Window) {
        // 全屏时的窗口几何是屏幕尺寸，写入设置会污染下次普通启动的窗口位置。
        if self.settings.window.remember_window_bounds && !window.is_fullscreen() {
            let bounds = window.bounds();
            self.settings.window.window_width = bounds.size.width / px(1.0);
            self.settings.window.window_height = bounds.size.height / px(1.0);
            self.settings.window.window_x = Some(bounds.origin.x / px(1.0));
            self.settings.window.window_y = Some(bounds.origin.y / px(1.0));
        }
        self.settings.window.previous_tab_sessions = self.tabs.iter().map(session_key).collect();
        self.settings.window.previous_tab_groups = self
            .groups
            .iter()
            .filter_map(|group| {
                let tab = self.tabs.iter().find(|tab| tab.id == group.active_pane)?;
                Some(SavedTabGroup {
                    session: session_key(tab),
                    layout: group.root.to_saved(),
                })
            })
            .collect();
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            // 这一处原先是静默丢弃的：保存失败等于下次启动窗口位置与标签组全丢，
            // 用户只会说"我上次开的那几个标签没了"，日志是唯一能回溯的现场。
            log::warn!("退出前保存窗口与标签组状态失败，下次启动将沿用上一次的记录，原因: {error}");
        }
    }

    /// 关闭程序前保存状态并退出
    pub(crate) fn close_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        log::info!(
            "应用正在退出，窗格数: {}，标签组数: {}",
            self.tabs.len(),
            self.groups.len()
        );
        self.save_window_state(window);
        cx.quit();
    }
}

/// 窗口恢复使用的会话键：本地终端固定为 `local`，其余使用持久化会话 ID。
pub(crate) fn session_key(tab: &TerminalTab) -> String {
    if matches!(tab.session.connection, Connection::Local(_)) {
        "local".to_string()
    } else {
        tab.session_id.to_string()
    }
}
