//! 审计查看器的外壳职责：打开入口与实体事件消费。
//!
//! 查看器状态、扫描与交互都在 `AuditViewerView`；外壳只计算当前会话的
//! 名称与主机（或标签页、会话目录中的目标），并把结果注入实体。实体上报的
//! 状态栏提示与错误在此落位，“打开设置”属于窗口级动作，也只能由外壳执行。

use super::entity::AuditViewerEvent;
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

    /// 消费查看器实体上报的意图。
    ///
    /// 事件自带全部数据，这里禁止回读实体，否则会在实体 `emit` 期间形成
    /// 双重租借；错误提示的日志已由实体在失败点记录，此处不再重复。
    pub(crate) fn on_audit_viewer_event(
        &mut self,
        event: &AuditViewerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AuditViewerEvent::Notice(message) => self.state.status = Some(message.clone()),
            AuditViewerEvent::Error(message) => self.state.set_error(message.clone()),
            AuditViewerEvent::OpenSettings => self.open_settings_modal(None, window, cx),
        }
        cx.notify();
    }
}
