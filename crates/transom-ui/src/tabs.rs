//! 会话增删改查、标签页激活/关闭/轮转与界面语言切换模块。

use crate::app::{AppShell, TabGroup, TabStatus};
use crate::palette::localized_error;
use crate::session_form::SessionForm;
use gpui_kit::component::set_locale;
use gpui_kit::*;
use transom_core::{
    AppCommand, Connection, ErrorCode, GroupId, SavedTabGroup, SecretKey, SecretKind, SessionId,
    TabId,
};

impl AppShell {
    /// 新建标签组并返回组 ID；调用方随后把首个窗格加入 `tabs`。
    pub(crate) fn create_group(&mut self, tab_id: TabId) -> GroupId {
        let group = TabGroup::new(tab_id);
        let id = group.id;
        self.groups.push(group);
        id
    }

    /// 根据窗口设置恢复一个标签组及其分屏窗格。
    ///
    /// 窗格数量限制在 1..=6：设置文件属于外部输入，避免损坏数据一次拉起
    /// 大量并发连接；串口等不支持多窗格的会话在首次分屏失败后停止扩张。
    pub(crate) fn restore_tab_group(
        &mut self,
        saved: &SavedTabGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_count = saved.panes.clamp(1, 6);
        if saved.session == "local" {
            self.open_local_terminal(window, cx);
        } else if self
            .tabs
            .iter()
            .any(|tab| tab.session_id.to_string() == saved.session)
        {
            // 同一会话已在更早恢复的标签组中打开，避免损坏设置重复拉起连接。
            return;
        } else if let Some(session) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id.to_string() == saved.session)
            .cloned()
        {
            self.connect_session(session.id, window, cx);
        } else {
            return;
        }
        let Some(group_index) = self.active_group_index() else {
            return;
        };
        for _ in 1..pane_count {
            let before = self.groups[group_index].panes.len();
            self.split_active_pane(saved.axis, window, cx);
            let after = self
                .groups
                .get(group_index)
                .map_or(before, |group| group.panes.len());
            if after == before {
                break;
            }
        }
        if let Some(group) = self.groups.get_mut(group_index) {
            group.axis = saved.axis;
        }
        cx.notify();
    }

    pub(crate) fn group_index_for_tab(&self, tab_id: TabId) -> Option<usize> {
        self.groups
            .iter()
            .position(|group| group.panes.contains(&tab_id))
    }

    pub(crate) fn active_group_index(&self) -> Option<usize> {
        self.active_tab
            .and_then(|tab_id| self.group_index_for_tab(tab_id))
    }

    /// 从所属标签组中摘除一个窗格，返回替换后的活动窗格。
    ///
    /// 仅处理组与窗格关系，不触碰 `tabs` 本身或发送断开命令，供关闭窗格、
    /// 删除会话等多条路径复用。
    fn detach_pane_from_group(&mut self, tab_id: TabId) -> Option<TabId> {
        let group_index = self.group_index_for_tab(tab_id)?;
        let group = &mut self.groups[group_index];
        let pane_index = group.panes.iter().position(|id| *id == tab_id)?;
        group.panes.remove(pane_index);
        let replacement = if group.panes.is_empty() {
            self.groups.remove(group_index);
            self.groups
                .get(group_index.saturating_sub(1))
                .or_else(|| self.groups.first())
                .map(|group| group.active_pane)
        } else {
            if group.active_pane == tab_id {
                let next = group.panes[pane_index.min(group.panes.len() - 1)];
                group.active_pane = next;
            }
            Some(group.active_pane)
        };
        if self.active_tab == Some(tab_id) {
            replacement
        } else {
            None
        }
    }

    pub(crate) fn open_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(SessionForm::new(window, cx, None, None));
        cx.notify();
    }

    pub(crate) fn open_edit_session(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
        else {
            return;
        };
        let folder_path = session
            .folder_id
            .and_then(|id| self.catalog.folder_path(id));
        let is_serial = matches!(session.connection, Connection::Serial(_));
        self.form = Some(SessionForm::new(window, cx, Some(session), folder_path));
        if is_serial {
            self.refresh_serial_ports(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn save_session_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let editing = form.base.is_some();
        let save_password = form.save_password;
        let save_passphrase = form.save_passphrase;
        let built = match form.build_session(cx) {
            Ok(built) => built,
            Err(error) => {
                form.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.form = None;

        let mut built = built;
        built.session.folder_id = built
            .folder_path
            .as_deref()
            .and_then(|path| self.catalog.ensure_folder_path(path));
        let password_key = SecretKey::new(built.session.id, SecretKind::Password);
        let passphrase_key = SecretKey::new(built.session.id, SecretKind::KeyPassphrase);
        if save_password {
            if let Some(password) = &built.password
                && let Err(error) = self
                    .services
                    .secret_store
                    .set(&password_key, password.as_str())
            {
                self.error = Some(localized_error(&error));
            }
        } else if let Err(error) = self.services.secret_store.delete(&password_key)
            && error.code != ErrorCode::SecretStoreUnavailable
        {
            self.error = Some(localized_error(&error));
        }
        if save_passphrase {
            if let Some(passphrase) = &built.key_passphrase
                && let Err(error) = self
                    .services
                    .secret_store
                    .set(&passphrase_key, passphrase.as_str())
            {
                self.error = Some(localized_error(&error));
            }
        } else if let Err(error) = self.services.secret_store.delete(&passphrase_key)
            && error.code != ErrorCode::SecretStoreUnavailable
        {
            self.error = Some(localized_error(&error));
        }

        let session_id = built.session.id;
        if editing {
            if let Some(existing) = self
                .catalog
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
            {
                *existing = built.session;
            }
        } else {
            self.catalog.sessions.push(built.session);
        }
        self.persist_catalog();
        if !editing {
            self.connect_session(session_id, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn delete_session_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.take() else {
            return;
        };
        let Some(base) = form.base else {
            return;
        };
        self.delete_session_by_id(base.id, cx);
    }

    pub(crate) fn delete_session_by_id(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        self.catalog
            .sessions
            .retain(|session| session.id != session_id);
        self.selected_sessions.remove(&session_id);
        let removed_tabs: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|tab| tab.session_id == session_id)
            .map(|tab| tab.id)
            .collect();
        for tab_id in &removed_tabs {
            let _ = self
                .services
                .command_tx
                .send(AppCommand::CloseTab { tab_id: *tab_id });
        }
        self.tabs.retain(|tab| tab.session_id != session_id);
        let mut replacement = None;
        for tab_id in removed_tabs {
            if let Some(next) = self.detach_pane_from_group(tab_id) {
                replacement = Some(next);
            }
        }
        if self
            .active_tab
            .is_none_or(|id| !self.tabs.iter().any(|tab| tab.id == id))
        {
            self.active_tab = replacement.or_else(|| self.groups.first().map(|g| g.active_pane));
        }
        let _ = self
            .services
            .secret_store
            .delete(&SecretKey::new(session_id, SecretKind::Password));
        let _ = self
            .services
            .secret_store
            .delete(&SecretKey::new(session_id, SecretKind::KeyPassphrase));
        self.persist_catalog();
        cx.notify();
    }

    pub(crate) fn duplicate_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(mut session) = self
            .catalog
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .cloned()
        {
            session.id = SessionId::new();
            session.name = format!("{} (Copy)", session.name);
            self.catalog.sessions.push(session);
            self.persist_catalog();
            cx.notify();
        }
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
        let others: Vec<GroupId> = self
            .groups
            .iter()
            .filter(|group| group.id != target_group_id)
            .map(|group| group.id)
            .collect();
        for group_id in others {
            self.close_group(group_id, window, cx);
        }
    }

    pub(crate) fn reconnect_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let session_id = self.tabs[index].session_id;
        let session = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .unwrap_or_else(|| self.tabs[index].session.clone());
        let tab = &mut self.tabs[index];
        tab.terminal = None;
        tab.sftp = None;
        tab._subscription = None;
        tab.connection_id = None;
        tab.forward_statuses.clear();
        tab.network_traffic = None;
        tab.status = TabStatus::Connecting;
        tab.title = session.name.clone();
        self.active_tab = Some(tab_id);
        if let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.groups[group_index].active_pane = tab_id;
        }
        self.start_connection(tab_id, session, window, cx);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = Some(tab_id);
        if let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.groups[group_index].active_pane = tab_id;
        }
        if let Some(terminal) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.terminal.as_ref())
        {
            let focus_handle = terminal.read(cx).focus_handle().clone();
            window.focus(&focus_handle, cx);
        }
        cx.notify();
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
        let group = self.groups.remove(index);
        for tab_id in &group.panes {
            let _ = self
                .services
                .command_tx
                .send(AppCommand::CloseTab { tab_id: *tab_id });
        }
        self.tabs.retain(|tab| !group.panes.contains(&tab.id));

        let active_removed = self.active_tab.is_some_and(|id| group.panes.contains(&id));
        if active_removed {
            self.active_tab = self
                .groups
                .get(index.saturating_sub(1))
                .or_else(|| self.groups.first())
                .map(|group| group.active_pane);
            if let Some(active) = self.active_tab {
                self.activate_tab(active, window, cx);
            }
        }
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
        if self.groups[group_index].panes.len() == 1 {
            let group_id = self.groups[group_index].id;
            self.close_group(group_id, window, cx);
            return;
        }

        let _ = self
            .services
            .command_tx
            .send(AppCommand::CloseTab { tab_id });
        self.tabs.retain(|tab| tab.id != tab_id);
        let replacement = self.detach_pane_from_group(tab_id);
        if let Some(next) = replacement {
            self.activate_tab(next, window, cx);
        }
        cx.notify();
    }

    /// 连接尚未建立时的静默移除，不向控制器发送关闭命令。
    pub(crate) fn remove_tab(&mut self, tab_id: TabId) {
        let was_active = self.active_tab == Some(tab_id);
        self.tabs.retain(|tab| tab.id != tab_id);
        let replacement = self.detach_pane_from_group(tab_id);
        if was_active {
            self.active_tab = replacement.or_else(|| self.groups.first().map(|g| g.active_pane));
        }
    }

    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.groups.is_empty() {
            return;
        }
        let current = self.active_group_index().unwrap_or(0);
        let delta = if forward { 1 } else { self.groups.len() - 1 };
        let next = (current + delta) % self.groups.len();
        let tab_id = self.groups[next].active_pane;
        self.activate_tab(tab_id, window, cx);
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
