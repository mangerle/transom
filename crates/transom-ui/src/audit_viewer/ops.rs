//! 审计日志查看器交互与 AppShell 状态流转操作。

use super::types::{AuditViewer, MAX_AUDIT_LOG_BYTES, find_session_audit_logs, read_log_content};
use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{Connection, Session, SessionId, TabId, paths};

impl AppShell {
    /// 打开审计日志查看器。
    pub(crate) fn open_audit_viewer(
        &mut self,
        session: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (session_name, host, target_title) = match session {
            Some(s) => {
                let h = match &s.connection {
                    Connection::Ssh(ssh) => ssh.host.clone(),
                    Connection::Telnet(telnet) => telnet.host.clone(),
                    Connection::Serial(serial) => serial.port.clone(),
                    Connection::Local(_) => "local".to_string(),
                    _ => "session".to_string(),
                };
                (
                    Some(s.name.clone()),
                    Some(h),
                    rust_i18n::t!("audit_viewer.for_session", name = s.name.clone()).to_string(),
                )
            }
            None => (None, None, rust_i18n::t!("audit_viewer.title").to_string()),
        };

        let audit_dir = paths::audit_dir();
        let entries = find_session_audit_logs(
            &audit_dir,
            session_name.as_deref(),
            host.as_deref(),
            session_name.is_none(),
        );

        let (content, is_truncated) = if let Some(first) = entries.first() {
            read_log_content(&first.path, MAX_AUDIT_LOG_BYTES)
        } else {
            (String::new(), false)
        };

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("audit_viewer.search_placeholder").to_string())
        });

        self.modals.audit_viewer = Some(AuditViewer {
            target_title,
            session_name,
            host,
            entries,
            selected_index: 0,
            content,
            is_truncated,
            search_input,
            show_all: session.is_none(),
        });
        cx.notify();
    }

    /// 打开指定标签页的审计日志查看器。
    pub(crate) fn open_audit_viewer_for_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.session.clone());
        if let Some(s) = session {
            self.open_audit_viewer(Some(&s), window, cx);
        } else {
            self.open_audit_viewer(None, window, cx);
        }
    }

    /// 打开指定会话的审计日志查看器。
    pub(crate) fn open_audit_viewer_for_session(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned();
        if let Some(s) = session {
            self.open_audit_viewer(Some(&s), window, cx);
        } else {
            self.open_audit_viewer(None, window, cx);
        }
    }

    /// 切换查看的审计日志文件。
    pub(crate) fn select_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.modals.audit_viewer.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }
        viewer.selected_index = index;
        let path = viewer.entries[index].path.clone();
        let (content, is_truncated) = read_log_content(&path, MAX_AUDIT_LOG_BYTES);
        viewer.content = content;
        viewer.is_truncated = is_truncated;
        cx.notify();
    }

    /// 切换“仅当前会话”与“全部会话”范围。
    pub(crate) fn toggle_audit_viewer_scope(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.modals.audit_viewer.as_mut() else {
            return;
        };
        viewer.show_all = !viewer.show_all;
        let audit_dir = paths::audit_dir();
        viewer.entries = find_session_audit_logs(
            &audit_dir,
            viewer.session_name.as_deref(),
            viewer.host.as_deref(),
            viewer.show_all,
        );
        viewer.selected_index = 0;
        let (content, is_truncated) = if let Some(first) = viewer.entries.first() {
            read_log_content(&first.path, MAX_AUDIT_LOG_BYTES)
        } else {
            (String::new(), false)
        };
        viewer.content = content;
        viewer.is_truncated = is_truncated;
        cx.notify();
    }

    /// 复制当前审计日志内容到剪贴板。
    pub(crate) fn copy_audit_viewer_content(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.modals.audit_viewer.as_ref() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(viewer.content.clone()));
        self.status = Some(rust_i18n::t!("audit_viewer.copied").to_string());
        cx.notify();
    }

    /// 删除指定的审计日志文件。
    pub(crate) fn delete_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.modals.audit_viewer.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }

        let entry = viewer.entries.remove(index);
        let log_path = entry.path.clone();
        if let Err(e) = std::fs::remove_file(&log_path) {
            log::error!("删除审计日志文件失败，路径: {:?}，原因: {e}", log_path);
            self.error = Some(rust_i18n::t!("audit_viewer.delete_failed").to_string());
        } else {
            self.status = Some(rust_i18n::t!("audit_viewer.deleted").to_string());
        }

        if viewer.entries.is_empty() {
            viewer.selected_index = 0;
            viewer.content.clear();
            viewer.is_truncated = false;
        } else {
            if viewer.selected_index >= viewer.entries.len() {
                viewer.selected_index = viewer.entries.len() - 1;
            }
            let path = viewer.entries[viewer.selected_index].path.clone();
            let (content, is_truncated) = read_log_content(&path, MAX_AUDIT_LOG_BYTES);
            viewer.content = content;
            viewer.is_truncated = is_truncated;
        }
        cx.notify();
    }
}
