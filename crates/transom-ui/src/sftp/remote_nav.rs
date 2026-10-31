//! 远端列导航与过滤动作：主远端与并列远端共用同一套路径跳转、过滤与同步开关。
//!
//! 从 `remote_actions` 拆出：文件操作入口与导航状态机关注点不同，分开后两个
//! 文件都能保持在工程规范的体量内，导航改动不需要穿越一长串弹窗动作。

use super::entity::SftpPanelView;
use crate::app::AppShell;
use crate::sftp::{SFTP_DIR_CACHE_TTL, SftpActivePane, SftpRemotePaneRef, parent_remote};
use gpui_kit::*;
use transom_core::TabId;

impl SftpPanelView {
    /// 进入目标列的目录：更新路径、命中短期缓存，然后后台刷新。
    pub(crate) fn navigate_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_local_target = if target == SftpRemotePaneRef::Main && self.state.sync_navigation {
            path.trim_end_matches('/')
                .rsplit_once('/')
                .map(|(_, name)| name)
                .filter(|name| !name.is_empty())
                .and_then(|name| {
                    self.state
                        .local
                        .entries
                        .iter()
                        .find(|entry| entry.is_dir && entry.name == name)
                        .map(|entry| entry.path.clone())
                })
        } else {
            None
        };
        if let Some(local_target) = sync_local_target {
            self.local_navigate_only(tab_id, local_target, window, cx);
        }

        // 即时路径更新与 SWR 短期缓存命中呈现（消除点击假死感）
        let mut update_path_input = None;
        if let Some(pane) = self.state.remote_pane_mut(target) {
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
        self.request_list_pane(tab_id, target, path, cx);
    }

    /// 目标列回到主目录。
    pub(crate) fn go_home_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_pane(tab_id, target, ".".to_string(), window, cx);
    }

    /// 目标列返回上一级。
    pub(crate) fn go_up_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((parent, sync_local_parent)) = self.state.remote_pane(target).map(|pane| {
            let remote_parent = parent_remote(&pane.path);
            let local_parent = if target == SftpRemotePaneRef::Main && self.state.sync_navigation {
                self.state.local.path.parent().map(|p| p.to_path_buf())
            } else {
                None
            };
            (remote_parent, local_parent)
        }) else {
            return;
        };
        if let Some(local_parent) = sync_local_parent {
            self.local_navigate_only(tab_id, local_parent, window, cx);
        }
        self.request_list_pane(tab_id, target, parent, cx);
    }

    /// 目标列按路径输入框进入目录。
    pub(crate) fn enter_path_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .state
            .remote_pane(target)
            .map(|pane| pane.path_input.read(cx).value().trim().to_string())
        else {
            return;
        };
        if !path.is_empty() {
            self.request_list_pane(tab_id, target, path, cx);
        }
    }

    /// 目标列刷新当前目录（清掉短期缓存后重新请求）。
    pub(crate) fn refresh_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.state.remote_pane(target).map(|pane| pane.path.clone()) else {
            return;
        };
        if let Some(pane) = self.state.remote_pane_mut(target) {
            pane.dir_cache.remove(&path);
        }
        self.request_list_pane(tab_id, target, path, cx);
    }

    /// 切换目标列的过滤栏。
    pub(crate) fn toggle_filter_pane(
        &mut self,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((show_filter, input)) = self.state.remote_pane_mut(target).map(|pane| {
            pane.show_filter = !pane.show_filter;
            (pane.show_filter, pane.filter_input.clone())
        }) else {
            return;
        };
        if show_filter {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// 切换目标列的隐藏文件显示。
    pub(crate) fn toggle_hidden_pane(&mut self, target: SftpRemotePaneRef, cx: &mut Context<Self>) {
        let Some(pane) = self.state.remote_pane_mut(target) else {
            return;
        };
        pane.show_hidden = !pane.show_hidden;
        let show_hidden = pane.show_hidden;
        self.state.set_status(if show_hidden {
            rust_i18n::t!("sftp.nav.hidden_shown").to_string()
        } else {
            rust_i18n::t!("sftp.nav.hidden_hidden").to_string()
        });
        cx.notify();
    }

    /// 切换本地列的隐藏文件显示。
    pub(crate) fn toggle_local_hidden(&mut self, cx: &mut Context<Self>) {
        self.state.local.show_hidden = !self.state.local.show_hidden;
        let show_hidden = self.state.local.show_hidden;
        self.state.set_status(if show_hidden {
            rust_i18n::t!("sftp.nav.hidden_shown").to_string()
        } else {
            rust_i18n::t!("sftp.nav.hidden_hidden").to_string()
        });
        cx.notify();
    }

    /// 切换"主远端导航联动本地"开关。
    pub(crate) fn toggle_sync_navigation(&mut self, cx: &mut Context<Self>) {
        self.state.sync_navigation = !self.state.sync_navigation;
        let enabled = self.state.sync_navigation;
        self.state.set_status(
            if enabled {
                rust_i18n::t!("sftp.remote.sync_nav_enabled")
            } else {
                rust_i18n::t!("sftp.remote.sync_nav_disabled")
            }
            .to_string(),
        );
        cx.notify();
    }

    /// 隐藏文件快捷键：按当前活动栏切换对应列的开关。
    pub(crate) fn on_toggle_hidden_action(&mut self, cx: &mut Context<Self>) {
        match self.state.active_pane {
            SftpActivePane::Local => self.toggle_local_hidden(cx),
            SftpActivePane::Remote => {
                if self.state.local.only {
                    self.toggle_local_hidden(cx);
                } else {
                    self.toggle_hidden_pane(SftpRemotePaneRef::Main, cx);
                }
            }
            SftpActivePane::SideRemote(connection_id) => {
                self.toggle_hidden_pane(SftpRemotePaneRef::Side(connection_id), cx);
            }
        }
    }

    /// 切换本地列的过滤栏。
    pub(crate) fn toggle_filter_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.local.show_filter = !self.state.local.show_filter;
        let show_filter = self.state.local.show_filter;
        let input = self.state.local.filter_input.clone();
        if show_filter {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// 过滤快捷键：按当前活动栏选择目标列的过滤输入框。
    pub(crate) fn on_focus_filter_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.state.active_pane {
            SftpActivePane::Local => None,
            SftpActivePane::Remote if !self.state.local.only => Some(SftpRemotePaneRef::Main),
            SftpActivePane::Remote => None,
            SftpActivePane::SideRemote(connection_id) => {
                Some(SftpRemotePaneRef::Side(connection_id))
            }
        };
        match target {
            Some(target) => self.toggle_filter_pane(target, window, cx),
            // 活动栏是本地时聚焦本地过滤框。
            None => self.toggle_filter_local(window, cx),
        }
    }
}

impl AppShell {
    pub(crate) fn sftp_navigate_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.navigate_pane(tab_id, target, path, window, cx)
            });
        }
    }

    pub(crate) fn sftp_go_home_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.go_home_pane(tab_id, target, window, cx));
        }
    }

    pub(crate) fn sftp_go_up_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.go_up_pane(tab_id, target, window, cx));
        }
    }

    pub(crate) fn sftp_enter_path_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.enter_path_pane(tab_id, target, cx));
        }
    }

    pub(crate) fn sftp_refresh_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.refresh_pane(tab_id, target, cx));
        }
    }

    pub(crate) fn sftp_toggle_filter_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.toggle_filter_pane(target, window, cx));
        }
    }

    pub(crate) fn sftp_toggle_hidden_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.toggle_hidden_pane(target, cx));
        }
    }

    pub(crate) fn sftp_toggle_local_hidden(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.toggle_local_hidden(cx));
        }
    }

    pub(crate) fn sftp_toggle_sync_navigation(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.toggle_sync_navigation(cx));
        }
    }

    pub(crate) fn sftp_toggle_filter_local(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.toggle_filter_local(window, cx));
        }
    }
}
