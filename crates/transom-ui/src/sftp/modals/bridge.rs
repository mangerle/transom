//! SFTP 模态弹窗的外壳桥接：统一管理实体创建、事件订阅、提交执行与关闭清理。
//!
//! 设计原理（Why）：七类弹窗视图本身全部为纯实体，不直接依赖 `AppShell`；
//! 外壳侧只通过本模块创建实体并响应事件（发命令、写设置、入队传输、置空关闭），
//! 避免打开/关闭逻辑散落在 `panel_events`、`events` 与 `dialog_host` 多处。

use super::chmod::SftpChmodModalParams;
use super::chmod::{SftpChmodModalEvent, SftpChmodModalView};
use super::confirm::{SftpConfirmModalEvent, SftpConfirmModalView};
use super::filter::{SftpFilterModalEvent, SftpFilterModalView};
use super::input::{SftpInputModalEvent, SftpInputModalParams, SftpInputModalView};
use super::properties::{SftpPropertiesModalEvent, SftpPropertiesModalView};
use super::state::{SftpModals, SftpPendingDelete};
use crate::app::{AppShell, app_settings, update_app_settings};
use crate::palette::localized_error;
use crate::sftp::{
    FileProperties, SftpSearchModalEvent, SftpSearchModalState, SftpSearchModalView,
    SftpSyncModalEvent, SftpSyncModalState, SftpSyncModalView,
};
use gpui_kit::component::input::TextareaState;
use gpui_kit::*;
use transom_core::SftpOperation;

impl SftpModals {
    pub(crate) fn open_confirm(
        &mut self,
        action: SftpPendingDelete,
        title: String,
        description: String,
        cx: &mut Context<AppShell>,
    ) {
        let modal = cx.new(|_| SftpConfirmModalView::new(action, title, description));
        let sub = cx.subscribe(
            &modal,
            |shell, view, event: &SftpConfirmModalEvent, cx| match event {
                SftpConfirmModalEvent::Confirm => {
                    let action = view.read(cx).action.clone();
                    shell.state.ui.modals.sftp.close_confirm();
                    cx.notify();
                    crate::sftp::execute_sftp_delete(&mut shell.state, action, cx);
                }
                SftpConfirmModalEvent::Cancel => {
                    shell.state.ui.modals.sftp.close_confirm();
                    cx.notify();
                }
            },
        );
        self.confirm = Some(modal);
        self.subs.confirm = Some(sub);
        cx.notify();
    }

    pub(crate) fn open_properties(
        &mut self,
        scope: String,
        properties: FileProperties,
        cx: &mut Context<AppShell>,
    ) {
        let modal = cx.new(|_| SftpPropertiesModalView::new(scope, properties));
        let sub = cx.subscribe(
            &modal,
            |shell, _, event: &SftpPropertiesModalEvent, cx| match event {
                SftpPropertiesModalEvent::Close => {
                    shell.state.ui.modals.sftp.close_properties();
                    cx.notify();
                }
            },
        );
        self.properties = Some(modal);
        self.subs.properties = Some(sub);
        cx.notify();
    }

    pub(crate) fn open_filter(
        &mut self,
        textarea: Entity<TextareaState>,
        cx: &mut Context<AppShell>,
    ) {
        let modal = cx.new(|_| SftpFilterModalView::new(textarea));
        let sub = cx.subscribe(
            &modal,
            |shell, _, event: &SftpFilterModalEvent, cx| match event {
                SftpFilterModalEvent::Save(text) => {
                    update_app_settings(cx, |s| s.sftp.transfer_filter = text.clone());
                    let settings_snapshot = app_settings(cx).clone();
                    match shell.state.persist_settings(&settings_snapshot) {
                        Ok(()) => {
                            shell.state.ui.status =
                                Some(rust_i18n::t!("sftp.filter_rules_saved").to_string());
                        }
                        Err(error) => {
                            shell.state.set_error(localized_error(&error));
                        }
                    }
                    shell.state.ui.modals.sftp.close_filter();
                    cx.notify();
                }
                SftpFilterModalEvent::Cancel => {
                    shell.state.ui.modals.sftp.close_filter();
                    cx.notify();
                }
            },
        );
        self.filter = Some(modal);
        self.subs.filter = Some(sub);
        cx.notify();
    }

    pub(crate) fn open_chmod(&mut self, params: SftpChmodModalParams, cx: &mut Context<AppShell>) {
        let SftpChmodModalParams {
            tab_id,
            connection_id,
            file_name,
            remote_path,
            permissions,
            octal_input,
        } = params;
        let modal = cx.new(|_| {
            SftpChmodModalView::new(
                tab_id,
                connection_id,
                file_name,
                remote_path,
                permissions,
                octal_input,
            )
        });
        let sub = cx.subscribe(
            &modal,
            |shell, _, event: &SftpChmodModalEvent, cx| match event {
                SftpChmodModalEvent::Apply {
                    tab_id,
                    connection_id,
                    remote_path,
                    permissions,
                } => {
                    shell.state.ui.modals.sftp.close_chmod();
                    cx.notify();
                    let Some(conn_id) = connection_id.or_else(|| {
                        shell
                            .state
                            .workspace
                            .tab(*tab_id)
                            .and_then(|tab| tab.connection_id())
                    }) else {
                        return;
                    };
                    crate::sftp::send_sftp_op_for_connection(
                        &mut shell.state,
                        *tab_id,
                        conn_id,
                        SftpOperation::Chmod {
                            path: remote_path.clone(),
                            permissions: *permissions,
                        },
                        cx,
                    );
                }
                SftpChmodModalEvent::Cancel => {
                    shell.state.ui.modals.sftp.close_chmod();
                    cx.notify();
                }
            },
        );
        self.chmod = Some(modal);
        self.subs.chmod = Some(sub);
        cx.notify();
    }

    pub(crate) fn open_input(
        &mut self,
        params: SftpInputModalParams,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        let modal = cx.new(|cx| SftpInputModalView::new(params, window, cx));
        let sub = cx.subscribe_in(
            &modal,
            window,
            |shell, view, event: &SftpInputModalEvent, window, cx| match event {
                SftpInputModalEvent::Submit {
                    tab_id,
                    connection_id,
                    target,
                    value,
                } => {
                    if let Err(err) = crate::sftp::file_ops::commit_sftp_input_value(
                        &mut shell.state,
                        crate::sftp::file_ops::SftpInputCommit {
                            tab_id: *tab_id,
                            connection_id: *connection_id,
                            target,
                            name: value,
                        },
                        window,
                        cx,
                    ) {
                        view.update(cx, |view, cx| view.set_error(err, window, cx));
                        return;
                    }
                    shell.state.ui.modals.sftp.close_input();
                    cx.notify();
                    crate::sftp::focus_active_sftp_panel(&shell.state, window, cx);
                }
                SftpInputModalEvent::Cancel => {
                    shell.state.ui.modals.sftp.close_input();
                    cx.notify();
                    crate::sftp::focus_active_sftp_panel(&shell.state, window, cx);
                }
            },
        );
        self.input = Some(modal);
        self.subs.input = Some(sub);
        cx.notify();
    }

    pub(crate) fn open_sync(&mut self, state: SftpSyncModalState, cx: &mut Context<AppShell>) {
        let modal = cx.new(|_| SftpSyncModalView::new(state));
        let sub = cx.subscribe(
            &modal,
            |shell, _, event: &SftpSyncModalEvent, cx| match event {
                SftpSyncModalEvent::Rescan(tab_id) => {
                    crate::sftp::sftp_open_sync_modal(&mut shell.state, *tab_id, cx);
                }
                SftpSyncModalEvent::Start {
                    tab_id,
                    local_root,
                    remote_root,
                    entries,
                } => {
                    let connection_id = shell
                        .state
                        .ui
                        .modals
                        .sftp
                        .sync
                        .as_ref()
                        .map(|m| m.read(cx).state.connection_id);
                    let started = crate::sftp::execute_sftp_sync(
                        &mut shell.state,
                        crate::sftp::SftpSyncExecute {
                            tab_id: *tab_id,
                            connection_id,
                            local_root,
                            remote_root,
                            entries,
                        },
                        cx,
                    );
                    if started {
                        shell.state.ui.modals.sftp.close_sync();
                        cx.notify();
                    }
                }
                SftpSyncModalEvent::Cancel => {
                    shell.state.ui.modals.sftp.close_sync();
                    cx.notify();
                }
            },
        );
        self.sync = Some(modal);
        self.subs.sync = Some(sub);
        cx.notify();
    }

    pub(crate) fn open_search(&mut self, state: SftpSearchModalState, cx: &mut Context<AppShell>) {
        let modal = cx.new(|_| SftpSearchModalView::new(state));
        let sub = cx.subscribe(
            &modal,
            |shell, _, event: &SftpSearchModalEvent, cx| match event {
                SftpSearchModalEvent::Execute {
                    tab_id,
                    connection_id,
                    query,
                } => {
                    let Some(conn_id) = connection_id.or_else(|| {
                        shell
                            .state
                            .workspace
                            .tab(*tab_id)
                            .and_then(|tab| tab.connection_id())
                    }) else {
                        return;
                    };
                    crate::sftp::send_sftp_op_for_connection(
                        &mut shell.state,
                        *tab_id,
                        conn_id,
                        SftpOperation::Search {
                            query: query.clone(),
                        },
                        cx,
                    );
                }
                SftpSearchModalEvent::Jump {
                    tab_id,
                    connection_id,
                    dir,
                } => {
                    shell.state.ui.modals.sftp.close_search();
                    cx.notify();
                    if let Some(conn_id) = connection_id {
                        crate::sftp::request_sftp_list_for_connection(
                            &mut shell.state,
                            *tab_id,
                            *conn_id,
                            dir.clone(),
                            cx,
                        );
                    } else {
                        crate::sftp::request_sftp_list(&mut shell.state, *tab_id, dir.clone(), cx);
                    }
                }
                SftpSearchModalEvent::Cancel => {
                    shell.state.ui.modals.sftp.close_search();
                    cx.notify();
                }
            },
        );
        self.search = Some(modal);
        self.subs.search = Some(sub);
        cx.notify();
    }
}
