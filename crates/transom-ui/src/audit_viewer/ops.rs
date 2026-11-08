//! 审计日志查看器交互与 AppShell 状态流转操作。

use super::types::{AuditViewer, MAX_AUDIT_LOG_BYTES, find_session_audit_logs, read_log_content};
use crate::app::AppShell;
use crate::app::spawn_blocking_local;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{Connection, Session, SessionId, TabId};
use transom_platform::paths;

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
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("audit_viewer.search_placeholder").to_string())
        });
        let show_all = session.is_none();
        self.state.modals.tools.audit_viewer = Some(AuditViewer {
            target_title,
            session_name: session_name.clone(),
            host: host.clone(),
            entries: Vec::new(),
            selected_index: 0,
            content: String::new(),
            is_truncated: false,
            search_input,
            show_all,
        });
        let scan_name = session_name.clone();
        let scan_host = host.clone();
        cx.spawn(async move |this, cx| {
            let Some((entries, content, is_truncated)) = spawn_blocking_local(move || {
                let entries = find_session_audit_logs(
                    &audit_dir,
                    scan_name.as_deref(),
                    scan_host.as_deref(),
                    show_all,
                );
                let (content, is_truncated) = entries.first().map_or_else(
                    || (String::new(), false),
                    |first| read_log_content(&first.path, MAX_AUDIT_LOG_BYTES),
                );
                (entries, content, is_truncated)
            })
            .await
            else {
                log::error!("扫描审计日志的线程创建失败，查看器保持空列表");
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(viewer) = this.state.modals.tools.audit_viewer.as_mut() {
                    viewer.entries = entries;
                    viewer.selected_index = 0;
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                }
                cx.notify();
            });
        })
        .detach();
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

    /// 切换查看的审计日志文件。
    pub(crate) fn select_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.modals.tools.audit_viewer.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }
        viewer.selected_index = index;
        let path = viewer.entries[index].path.clone();
        cx.spawn(async move |this, cx| {
            let Some((content, is_truncated)) =
                spawn_blocking_local(move || read_log_content(&path, MAX_AUDIT_LOG_BYTES)).await
            else {
                log::error!("读取审计日志的线程创建失败");
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(viewer) = this.state.modals.tools.audit_viewer.as_mut()
                    && viewer.selected_index == index
                {
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 切换“仅当前会话”与“全部会话”范围。
    pub(crate) fn toggle_audit_viewer_scope(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.modals.tools.audit_viewer.as_mut() else {
            return;
        };
        viewer.show_all = !viewer.show_all;
        let show_all = viewer.show_all;
        let session_name = viewer.session_name.clone();
        let host = viewer.host.clone();
        viewer.entries.clear();
        viewer.selected_index = 0;
        viewer.content.clear();
        viewer.is_truncated = false;
        let audit_dir = paths::audit_dir();
        cx.spawn(async move |this, cx| {
            let Some((entries, content, is_truncated)) = spawn_blocking_local(move || {
                let entries = find_session_audit_logs(
                    &audit_dir,
                    session_name.as_deref(),
                    host.as_deref(),
                    show_all,
                );
                let (content, is_truncated) = entries.first().map_or_else(
                    || (String::new(), false),
                    |first| read_log_content(&first.path, MAX_AUDIT_LOG_BYTES),
                );
                (entries, content, is_truncated)
            })
            .await
            else {
                log::error!("切换审计范围时扫描线程创建失败");
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(viewer) = this.state.modals.tools.audit_viewer.as_mut()
                    && viewer.show_all == show_all
                {
                    viewer.entries = entries;
                    viewer.selected_index = 0;
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 复制当前审计日志内容到剪贴板。
    pub(crate) fn copy_audit_viewer_content(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.modals.tools.audit_viewer.as_ref() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(viewer.content.clone()));
        self.state.status = Some(rust_i18n::t!("audit_viewer.copied").to_string());
        cx.notify();
    }

    /// 删除指定的审计日志文件。
    pub(crate) fn delete_audit_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.modals.tools.audit_viewer.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }

        let entry = viewer.entries.remove(index);
        let log_path = entry.path.clone();
        if viewer.entries.is_empty() {
            viewer.selected_index = 0;
            viewer.content.clear();
            viewer.is_truncated = false;
        } else {
            if viewer.selected_index >= viewer.entries.len() {
                viewer.selected_index = viewer.entries.len() - 1;
            }
            viewer.content.clear();
            viewer.is_truncated = false;
        }
        let next_path = viewer
            .entries
            .get(viewer.selected_index)
            .map(|entry| entry.path.clone());
        cx.spawn(async move |this, cx| {
            let Some(removed) = spawn_blocking_local(move || std::fs::remove_file(&log_path)).await
            else {
                log::error!("删除审计日志的线程创建失败");
                return;
            };
            let read = if let Some(path) = next_path {
                let read_path = path.clone();
                let Some((content, is_truncated)) =
                    spawn_blocking_local(move || read_log_content(&read_path, MAX_AUDIT_LOG_BYTES))
                        .await
                else {
                    log::error!("删除审计日志后读取下一个文件的线程创建失败");
                    return;
                };
                Some((path, content, is_truncated))
            } else {
                None
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(viewer) = this.state.modals.tools.audit_viewer.as_mut()
                    && let Some((path, content, is_truncated)) = read
                    && viewer
                        .entries
                        .get(viewer.selected_index)
                        .is_some_and(|entry| entry.path == path)
                {
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                }
                match removed {
                    Ok(()) => {
                        this.state.status = Some(rust_i18n::t!("audit_viewer.deleted").to_string())
                    }
                    Err(error) => {
                        log::error!("删除审计日志文件失败，原因: {error}");
                        this.state
                            .set_error(rust_i18n::t!("audit_viewer.delete_failed"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
