//! 窗口几何与标签组状态的持久化。

use crate::app::{AppShell, TerminalTab};
use gpui_kit::*;
use transom_core::{Connection, SavedTabGroup, SavedTabTiling};

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
        // 会话列表展开状态与标签组、窗口几何同属"上次状态"，一起在退出时落盘：
        // 崩溃后三者一起退到上次正常退出，不会出现标签组是旧的、侧栏是新的那种
        // 半新半旧；也不必为一次点击写一次配置文件。
        self.settings.window.last_sidebar_visible = self.sidebar_visible;
        self.settings.window.previous_tab_sessions =
            self.workspace.tabs.iter().map(session_key).collect();
        self.prune_tilings();
        // 先建立"组 ID → 落盘下标"的映射：并排快照用下标引用标签组，
        // 而组 ID 是运行期 UUID，不随设置文件保存。
        let saved_groups: Vec<_> = self
            .workspace
            .groups
            .iter()
            .filter_map(|group| {
                let tab = self
                    .workspace
                    .tabs
                    .iter()
                    .find(|tab| tab.id == group.active_pane)?;
                Some((
                    group.id,
                    SavedTabGroup {
                        session: session_key(tab),
                        layout: group.root.to_saved(),
                    },
                ))
            })
            .collect();
        // 每张并排网格各自落盘：槽位按"组 ID → 落盘下标"引用标签组，
        // 而组 ID 是运行期 UUID，不随设置文件保存。
        self.settings.window.tab_tilings = self
            .workspace
            .tiled_sets
            .iter()
            .filter_map(|tiled| {
                let slots: Vec<Option<usize>> = tiled
                    .slots
                    .iter()
                    .map(|slot| {
                        slot.and_then(|group| saved_groups.iter().position(|(id, _)| *id == group))
                    })
                    .collect();
                // 没落盘的标签组会变成空槽；有效槽位少于两个时并排本身已没有意义。
                if slots.iter().flatten().count() < 2 {
                    return None;
                }
                Some(SavedTabTiling {
                    rows: tiled.rows,
                    cols: tiled.cols,
                    slots,
                    focused: tiled.focused,
                    auto: tiled.auto,
                })
            })
            .collect();
        self.settings.window.previous_tab_groups =
            saved_groups.into_iter().map(|(_, group)| group).collect();
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
            self.workspace.tabs.len(),
            self.workspace.groups.len()
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
