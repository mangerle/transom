//! 会话删除与副本复制操作。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::sftp::PendingEditorClose;
use gpui_kit::*;
use transom_core::{ErrorCode, SecretKey, SecretKind, SessionId, TabId, TerminalCommand};

impl AppShell {
    pub(crate) fn delete_session_by_id(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        // 会话下任一标签存在未保存的编辑面板时，删除会被闸门挂起，
        // 待用户处理完毕后续行（同关闭标签组的保护水位）。
        let removed_tabs: Vec<TabId> = self
            .workspace
            .tabs
            .iter()
            .filter(|tab| tab.session_id == session_id)
            .map(|tab| tab.id)
            .collect();
        if self.block_close_for_dirty_editors(
            &removed_tabs,
            PendingEditorClose::Session(session_id),
            cx,
        ) {
            return;
        }
        self.catalog
            .sessions
            .retain(|session| session.id != session_id);
        self.sidebar.selected_sessions.remove(&session_id);
        self.settings
            .window
            .recent_sessions
            .retain(|id| *id != session_id);
        for tab_id in &removed_tabs {
            self.send_command(TerminalCommand::CloseTab { tab_id: *tab_id });
        }
        self.workspace
            .tabs
            .retain(|tab| tab.session_id != session_id);
        self.prune_editor_panels();
        let tiled_focus = self.active_tiling().and_then(|tiled| tiled.focused_group());
        let mut replacement = None;
        for tab_id in removed_tabs {
            if let Some(next) = self.detach_pane_from_group(tab_id) {
                replacement = Some(next);
            }
        }
        self.prune_tilings();
        if self
            .workspace
            .active_tab
            .is_none_or(|id| !self.workspace.has_tab(id))
        {
            self.workspace.active_tab = self.fallback_active_pane(replacement, tiled_focus);
        }
        self.prune_tab_selection();
        self.sync_broadcast_targets();
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
        let Some(mut session) = self
            .catalog
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .cloned()
        else {
            return;
        };
        let new_id = SessionId::new();
        let name = session.name.clone();
        session.id = new_id;
        session.name = self.unique_duplicate_name(&name);
        self.catalog.sessions.push(session);
        self.copy_secrets(session_id, new_id);
        self.persist_catalog();
        cx.notify();
    }

    /// 为副本挑一个目录里未被占用的名字：`X 副本`、`X 副本 #1`、`X 副本 #2`...
    fn unique_duplicate_name(&self, original: &str) -> String {
        let base = duplicate_base(original);
        let first = rust_i18n::t!("common.duplicate_name", name = base).to_string();
        let taken = |candidate: &str| self.catalog.sessions.iter().any(|s| s.name == candidate);
        if !taken(&first) {
            return first;
        }
        let limit = self.catalog.sessions.len() + 2;
        (1..=limit)
            .map(|index| format!("{first} #{index}"))
            .find(|candidate| !taken(candidate))
            .unwrap_or_else(|| format!("{first} #{}", limit + 1))
    }

    /// 把一个会话名下的钥匙串条目复制到另一个会话 ID。
    fn copy_secrets(&mut self, from: SessionId, to: SessionId) {
        for kind in [SecretKind::Password, SecretKind::KeyPassphrase] {
            let source = SecretKey::new(from, kind);
            let target = SecretKey::new(to, kind);
            match self.services.secret_store.get(&source) {
                Ok(Some(secret)) => {
                    if let Err(error) = self.services.secret_store.set(&target, &secret)
                        && error.code != ErrorCode::SecretStoreUnavailable
                    {
                        self.set_error(localized_error(&error));
                    }
                }
                Ok(None) => {}
                Err(error) if error.code == ErrorCode::SecretStoreUnavailable => {}
                Err(error) => self.set_error(localized_error(&error)),
            }
        }
    }
}

/// 计算副本命名基准：剥掉我们自己加上的"副本"后缀、以及紧随其后的 ` #N` 编号。
pub(super) fn duplicate_base(name: &str) -> String {
    let suffix = rust_i18n::t!("common.duplicate_name", name = "").to_string();
    let trimmed = name.trim_end();
    if let Some(base) = strip_copy_suffix(trimmed, &suffix) {
        return base.to_string();
    }
    if let Some(position) = trimmed.rfind(" #")
        && let Some(tail) = trimmed.get(position + 2..)
        && !tail.is_empty()
        && tail.chars().all(|c| c.is_ascii_digit())
        && let Some(base) = strip_copy_suffix(trimmed[..position].trim_end(), &suffix)
    {
        return base.to_string();
    }
    trimmed.to_string()
}

/// 去掉本地化的"副本"后缀；不是我们生成的后缀就返回 None，不猜测用户命名。
pub(super) fn strip_copy_suffix<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    text.strip_suffix(suffix).map(str::trim_end)
}
