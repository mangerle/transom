//! 审计日志查看器的外壳入口与临时委派方法。
//!
//! 打开状态、扫描与交互都归 [`AuditViewerView`]；本文件只计算会话信息并转发。
//! 渲染迁移期间，`select/toggle/delete/copy` 暂留这里供旧视图调用，最终会随
//! `view.rs` 一起删除。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{Connection, Session, SessionId, TabId};

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
        let show_all = session.is_none();
        let view = self.state.modals.tools.audit_viewer.clone();
        view.update(cx, |view, cx| {
            view.open(target_title, session_name, host, show_all, window, cx);
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
            .state
            .workspace
            .tab(tab_id)
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
            .state
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

    /// 委派：切换查看的审计日志文件。
    pub(crate) fn select_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.audit_viewer.clone();
        view.update(cx, |view, cx| view.select(index, cx));
    }

    /// 委派：切换“仅当前会话”与“全部会话”范围。
    pub(crate) fn toggle_audit_viewer_scope(&mut self, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.audit_viewer.clone();
        view.update(cx, |view, cx| view.toggle_scope(cx));
    }

    /// 委派：复制当前审计日志内容到剪贴板。
    pub(crate) fn copy_audit_viewer_content(&mut self, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.audit_viewer.clone();
        view.update(cx, |view, cx| view.copy_content(cx));
    }

    /// 委派：删除指定的审计日志文件。
    pub(crate) fn delete_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.audit_viewer.clone();
        view.update(cx, |view, cx| view.delete(index, cx));
    }
}
