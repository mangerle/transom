//! 远端列导航与过滤动作：主远端与并列远端共用同一套路径跳转、过滤与同步开关。
//!
//! 从 `remote_actions` 拆出：文件操作入口与导航状态机关注点不同，分开后两个
//! 文件都能保持在工程规范的体量内，导航改动不需要穿越一长串弹窗动作。

use crate::app::AppShell;
use crate::keymap;
use crate::sftp::{SFTP_DIR_CACHE_TTL, SftpActivePane, SftpRemotePaneRef, parent_remote};
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 进入目标列的目录：更新路径、命中短期缓存，然后后台刷新。
    pub(crate) fn sftp_navigate_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_local_target = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| {
                // 同步导航只属于"主远端 ↔ 本地"；并列列导航不牵动本地。
                if target != SftpRemotePaneRef::Main || !panel.sync_navigation {
                    return None;
                }
                let child_name = path
                    .trim_end_matches('/')
                    .rsplit_once('/')
                    .map(|(_, n)| n)?;
                if child_name.is_empty() {
                    return None;
                }
                panel
                    .local
                    .entries
                    .iter()
                    .find(|e| e.is_dir && e.name == child_name)
                    .map(|e| e.path.clone())
            });
        if let Some(local_target) = sync_local_target {
            self.sftp_local_navigate_only(tab_id, local_target, window, cx);
        }

        // 即时路径更新与 SWR 短期缓存命中呈现（消除点击假死感）
        let mut update_path_input = None;
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(pane) = panel.remote_pane_mut(target)
        {
            pane.path = path.clone();
            pane.loading = true;
            pane.error = None;
            if let Some(cached) = pane.dir_cache.get(&path)
                && cached.cached_at.elapsed() < SFTP_DIR_CACHE_TTL
            {
                pane.entries = cached.entries.clone();
                let valid_paths = pane
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                pane.sync_selection(&valid_paths);
            }
            update_path_input = Some((pane.path_input.clone(), path.clone()));
        }
        if let Some((input, path_str)) = update_path_input {
            input.update(cx, |state, cx| state.set_value(path_str, window, cx));
        }

        // 后台发起真实网络验证（Stale-While-Revalidate）
        self.request_sftp_list_pane(tab_id, target, path, cx);
    }

    /// 目标列回到主目录。
    pub(crate) fn sftp_go_home_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sftp_navigate_pane(tab_id, target, ".".to_string(), window, cx);
    }

    /// 目标列返回上一级。
    pub(crate) fn sftp_go_up_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((parent, sync_local_parent)) = self.workspace.tab(tab_id).and_then(|tab| {
            let panel = tab.sftp()?;
            let pane = panel.remote_pane(target)?;
            let remote_parent = parent_remote(&pane.path);
            let local_parent = if target == SftpRemotePaneRef::Main && panel.sync_navigation {
                panel.local.path.parent().map(|p| p.to_path_buf())
            } else {
                None
            };
            Some((remote_parent, local_parent))
        }) else {
            return;
        };
        if let Some(local_parent) = sync_local_parent {
            self.sftp_local_navigate_only(tab_id, local_parent, window, cx);
        }
        self.request_sftp_list_pane(tab_id, target, parent, cx);
    }

    /// 目标列按路径输入框进入目录。
    pub(crate) fn sftp_enter_path_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| panel.remote_pane(target))
            .map(|pane| pane.path_input.read(cx).value().trim().to_string())
        else {
            return;
        };
        if !path.is_empty() {
            self.request_sftp_list_pane(tab_id, target, path, cx);
        }
    }

    /// 目标列刷新当前目录（清掉短期缓存后重新请求）。
    pub(crate) fn sftp_refresh_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| panel.remote_pane(target))
            .map(|pane| pane.path.clone())
        else {
            return;
        };
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(pane) = panel.remote_pane_mut(target)
        {
            pane.dir_cache.remove(&path);
        }
        self.request_sftp_list_pane(tab_id, target, path, cx);
    }

    /// 切换目标列的过滤栏。
    pub(crate) fn sftp_toggle_filter_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((show_filter, input)) = self.sftp_panel_mut_tab(tab_id).and_then(|panel| {
            let pane = panel.remote_pane_mut(target)?;
            pane.show_filter = !pane.show_filter;
            Some((pane.show_filter, pane.filter_input.clone()))
        }) else {
            return;
        };
        if show_filter {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        } else {
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        cx.notify();
    }

    /// 切换目标列的隐藏文件显示。
    pub(crate) fn sftp_toggle_hidden_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(show_hidden) = self.sftp_panel_mut_tab(tab_id).and_then(|panel| {
            let pane = panel.remote_pane_mut(target)?;
            pane.show_hidden = !pane.show_hidden;
            Some(pane.show_hidden)
        }) else {
            return;
        };
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.set_status(if show_hidden {
                rust_i18n::t!("sftp.nav.hidden_shown").to_string()
            } else {
                rust_i18n::t!("sftp.nav.hidden_hidden").to_string()
            });
            cx.notify();
        }
    }

    /// 切换本地列的隐藏文件显示。
    pub(crate) fn sftp_toggle_local_hidden(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.local.show_hidden = !panel.local.show_hidden;
            panel.set_status(if panel.local.show_hidden {
                rust_i18n::t!("sftp.nav.hidden_shown").to_string()
            } else {
                rust_i18n::t!("sftp.nav.hidden_hidden").to_string()
            });
            cx.notify();
        }
    }

    /// 切换"主远端导航联动本地"开关（面板级，只有主远端显示该按钮）。
    pub(crate) fn sftp_toggle_sync_navigation(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.sync_navigation = !panel.sync_navigation;
            panel.set_status(
                if panel.sync_navigation {
                    rust_i18n::t!("sftp.remote.sync_nav_enabled")
                } else {
                    rust_i18n::t!("sftp.remote.sync_nav_disabled")
                }
                .to_string(),
            );
            cx.notify();
        }
    }

    /// 隐藏文件快捷键：按当前活动栏切换对应列的开关。
    pub(crate) fn on_toggle_sftp_hidden_pane(
        &mut self,
        _: &keymap::ToggleSftpHidden,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.workspace.active_tab else {
            return;
        };
        let Some(active) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| panel.active_pane)
        else {
            return;
        };
        match active {
            SftpActivePane::Local => self.sftp_toggle_local_hidden(tab_id, cx),
            SftpActivePane::Remote => {
                let is_local_only = self
                    .workspace
                    .tab(tab_id)
                    .and_then(|tab| tab.sftp())
                    .is_some_and(|panel| panel.local.only);
                if is_local_only {
                    self.sftp_toggle_local_hidden(tab_id, cx);
                } else {
                    self.sftp_toggle_hidden_pane(tab_id, SftpRemotePaneRef::Main, cx);
                }
            }
            SftpActivePane::SideRemote(connection_id) => {
                self.sftp_toggle_hidden_pane(tab_id, SftpRemotePaneRef::Side(connection_id), cx);
            }
        }
    }

    /// 切换本地列的过滤栏。
    pub(crate) fn sftp_toggle_filter_local(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((show_filter, input)) = self.sftp_panel_mut_tab(tab_id).map(|panel| {
            panel.local.show_filter = !panel.local.show_filter;
            (panel.local.show_filter, panel.local.filter_input.clone())
        }) else {
            return;
        };
        if show_filter {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        } else {
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        cx.notify();
    }

    /// 过滤快捷键：按当前活动栏选择目标列的过滤输入框。
    pub(crate) fn on_focus_sftp_filter_pane(
        &mut self,
        _: &keymap::FocusSftpFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.workspace.active_tab else {
            return;
        };
        let target = self.workspace.tab(tab_id).and_then(|tab| {
            let panel = tab.sftp()?;
            match panel.active_pane {
                SftpActivePane::Local => None,
                SftpActivePane::Remote if !panel.local.only => Some(SftpRemotePaneRef::Main),
                SftpActivePane::Remote => None,
                SftpActivePane::SideRemote(connection_id) => {
                    Some(SftpRemotePaneRef::Side(connection_id))
                }
            }
        });
        match target {
            Some(target) => self.sftp_toggle_filter_pane(tab_id, target, window, cx),
            // 活动栏是本地时聚焦本地过滤框。
            None => self.sftp_toggle_filter_local(tab_id, window, cx),
        }
    }
}
