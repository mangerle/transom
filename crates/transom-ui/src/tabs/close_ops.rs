//! 标签关闭、窗格摘除与应用语言切换操作。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::pane_tree::RemoveOutcome;
use crate::sftp::PendingEditorClose;
use crate::tiling::StripEntry;
use gpui_kit::component::set_locale;
use gpui_kit::*;
use transom_core::{AppCommand, GroupId, TabId};

impl AppShell {
    /// 该窗格此刻是否正被渲染：属于可见标签组、在分割树里，且没被"单窗格最大化"挡在后面。
    pub(crate) fn is_rendered_pane(&self, tab_id: TabId) -> bool {
        let Some(group_index) = self.group_index_for_tab(tab_id) else {
            return false;
        };
        let group = &self.groups[group_index];
        let visible = match self.active_tiling() {
            Some(tiled) => tiled.contains(group.id),
            None => self.active_group_index() == Some(group_index),
        };
        if !visible {
            return false;
        }
        match group.zoomed {
            Some(zoomed) => zoomed == tab_id,
            None => group.root.contains(tab_id),
        }
    }

    pub(crate) fn active_group_index(&self) -> Option<usize> {
        self.active_tab
            .and_then(|tab_id| self.group_index_for_tab(tab_id))
    }

    /// 从所属标签组的分割树里摘除一个窗格，返回应接管的焦点窗格。
    pub(super) fn detach_pane_from_group(&mut self, tab_id: TabId) -> Option<TabId> {
        let group_index = self.group_index_for_tab(tab_id)?;
        let was_active = self.active_tab == Some(tab_id);
        let replacement = match self.groups[group_index].root.remove_leaf(tab_id) {
            RemoveOutcome::Empty => {
                self.groups.remove(group_index);
                if was_active {
                    self.groups
                        .get(group_index.saturating_sub(1))
                        .or_else(|| self.groups.first())
                        .map(|group| group.active_pane)
                } else {
                    None
                }
            }
            RemoveOutcome::Removed { next_focus } => {
                let group = &mut self.groups[group_index];
                if group.active_pane == tab_id {
                    group.active_pane = next_focus;
                }
                if group.zoomed == Some(tab_id) {
                    group.zoomed = None;
                }
                if was_active { Some(next_focus) } else { None }
            }
        };
        self.prune_pane_split_states();
        replacement
    }

    pub(crate) fn close_other_tabs(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_group) = self.group_index_for_tab(tab_id) else {
            return;
        };
        let target_group_id = self.groups[target_group].id;
        let mut keep = vec![target_group_id];
        if let Some(tiled) = self.tiling_of_group(target_group_id) {
            keep.extend(tiled.occupied_groups());
        }
        let others: Vec<GroupId> = self
            .groups
            .iter()
            .filter(|group| !keep.contains(&group.id))
            .map(|group| group.id)
            .collect();
        for group_id in others {
            self.close_group(group_id, window, cx);
        }
    }

    /// 关闭窗格所属的整个标签组。
    pub(crate) fn close_tab(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self
            .group_index_for_tab(tab_id)
            .map(|index| self.groups[index].id)
        else {
            return;
        };
        self.close_group(group_id, window, cx);
    }

    /// 关闭标签组，并回收其中全部窗格的连接。
    pub(crate) fn close_group(
        &mut self,
        group_id: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.groups.iter().position(|group| group.id == group_id) else {
            return;
        };
        let panes = self.groups[index].pane_ids();
        if self.block_close_for_dirty_editors(&panes, PendingEditorClose::Group(group_id), cx) {
            return;
        }
        let tiled_focus = self
            .tiling_of_group(group_id)
            .and_then(|tiled| tiled.focused_group());
        self.groups.remove(index);
        for tab_id in &panes {
            self.send_command(AppCommand::CloseTab { tab_id: *tab_id });
        }
        self.tabs.retain(|tab| !panes.contains(&tab.id));
        self.prune_pane_split_states();
        self.prune_editor_panels();
        self.tab_selection.remove(&group_id);
        self.prune_tilings();

        let active_removed = self.active_tab.is_some_and(|id| panes.contains(&id));
        if active_removed {
            let tiled_return = self
                .active_tiling()
                .and_then(|tiled| tiled.focused_group())
                .or(tiled_focus)
                .and_then(|id| self.groups.iter().find(|group| group.id == id))
                .map(|group| group.active_pane);
            self.active_tab = tiled_return.or_else(|| {
                self.groups
                    .get(index.saturating_sub(1))
                    .or_else(|| self.groups.first())
                    .map(|group| group.active_pane)
            });
            if let Some(active) = self.active_tab {
                self.activate_tab(active, window, cx);
            }
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
        if self.groups.is_empty() && self.settings.window.close_window_on_all_tabs_closed {
            self.close_app(window, cx);
            return;
        }
        cx.notify();
    }

    /// 关闭单个窗格；组内最后一个窗格关闭时等价于关闭整个标签。
    pub(crate) fn close_pane(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group_index) = self.group_index_for_tab(tab_id) else {
            return;
        };
        if self.groups[group_index].pane_count() == 1 {
            let group_id = self.groups[group_index].id;
            self.close_group(group_id, window, cx);
            return;
        }
        if self.block_close_for_dirty_editors(
            std::slice::from_ref(&tab_id),
            PendingEditorClose::Pane(tab_id),
            cx,
        ) {
            return;
        }

        self.send_command(AppCommand::CloseTab { tab_id });
        self.tabs.retain(|tab| tab.id != tab_id);
        self.prune_editor_panels();
        let replacement = self.detach_pane_from_group(tab_id);
        self.prune_tilings();
        if let Some(next) = replacement {
            self.activate_tab(next, window, cx);
        }
        cx.notify();
    }

    /// 连接尚未建立时的静默移除，不向控制器发送关闭命令。
    pub(crate) fn remove_tab(&mut self, tab_id: TabId) {
        let was_active = self.active_tab == Some(tab_id);
        let group_id = self
            .group_index_for_tab(tab_id)
            .map(|index| self.groups[index].id);
        let tiled_focus = group_id
            .and_then(|group| self.tiling_of_group(group))
            .and_then(|tiled| tiled.focused_group());
        self.tabs.retain(|tab| tab.id != tab_id);
        self.prune_editor_panels();
        let replacement = self.detach_pane_from_group(tab_id);
        self.prune_tilings();
        if was_active {
            self.active_tab = replacement
                .or_else(|| {
                    self.active_tiling()
                        .and_then(|tiled| tiled.focused_group())
                        .and_then(|group| {
                            self.groups
                                .iter()
                                .find(|item| item.id == group)
                                .map(|item| item.active_pane)
                        })
                })
                .or_else(|| {
                    tiled_focus.and_then(|group| {
                        self.groups
                            .iter()
                            .find(|item| item.id == group)
                            .map(|item| item.active_pane)
                    })
                })
                .or_else(|| self.groups.first().map(|group| group.active_pane));
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
    }

    /// 在标签栏条目之间轮转：并排网格算一条，组内成员不单独占位。
    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.strip_entries();
        if entries.len() <= 1 {
            return;
        }
        let current = self
            .active_group_id()
            .and_then(|group| {
                entries
                    .iter()
                    .position(|entry| self.entry_contains(*entry, group))
            })
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % entries.len()
        } else {
            (current + entries.len() - 1) % entries.len()
        };
        let target = match entries[next] {
            StripEntry::Single(group) => Some(group),
            StripEntry::Tiled { id, .. } => self
                .tiling_by_id(id)
                .and_then(|tiled| tiled.focused_group()),
        };
        if let Some(group) = target {
            self.activate_group(group, window, cx);
        }
    }

    pub(crate) fn toggle_language(&mut self, cx: &mut Context<Self>) {
        self.settings.language = if self.settings.language == "zh-CN" {
            "en".to_string()
        } else {
            "zh-CN".to_string()
        };
        set_locale(&self.settings.language);
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        }
        cx.notify();
    }
}
