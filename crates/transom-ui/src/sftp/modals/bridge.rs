//! SFTP 模态弹窗的外壳桥接：统一管理实体创建、事件订阅、提交执行与关闭清理。
//!
//! 设计原理（Why）：七类弹窗视图本身全部为纯实体，不直接依赖 `AppShell`；
//! 外壳侧只通过本模块创建实体并响应事件（发命令、写设置、入队传输、置空关闭），
//! 避免打开/关闭逻辑散落在 `panel_events`、`events` 与 `dialog_host` 多处。

use super::chmod::{SftpChmodModalEvent, SftpChmodModalView};
use super::confirm::{SftpConfirmModalEvent, SftpConfirmModalView};
use super::filter::{SftpFilterModalEvent, SftpFilterModalView};
use super::input::{SftpInputModalEvent, SftpInputModalView};
use super::properties::{SftpPropertiesModalEvent, SftpPropertiesModalView};
use super::state::SftpPendingDelete;
use crate::app::{AppShell, app_settings, update_app_settings};
use crate::palette::localized_error;
use crate::sftp::{
    FileProperties, SftpInputModalParams, SftpSearchModalEvent, SftpSearchModalState,
    SftpSearchModalView, SftpSyncModalEvent, SftpSyncModalState, SftpSyncModalView,
};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use transom_core::{ConnectionId, SftpOperation, TabId};

pub(crate) fn close_sftp_confirm_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.confirm = None;
    shell.state.sftp_modals.subs.confirm = None;
    cx.notify();
}

pub(crate) fn close_sftp_input_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.input = None;
    shell.state.sftp_modals.subs.input = None;
    cx.notify();
}

pub(crate) fn close_sftp_chmod_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.chmod = None;
    shell.state.sftp_modals.subs.chmod = None;
    cx.notify();
}

pub(crate) fn close_sftp_properties_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.properties = None;
    shell.state.sftp_modals.subs.properties = None;
    cx.notify();
}

pub(crate) fn close_sftp_filter_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.filter = None;
    shell.state.sftp_modals.subs.filter = None;
    cx.notify();
}

pub(crate) fn close_sftp_sync_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.sync = None;
    shell.state.sftp_modals.subs.sync = None;
    cx.notify();
}

pub(crate) fn close_sftp_search_modal(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell.state.sftp_modals.search = None;
    shell.state.sftp_modals.subs.search = None;
    cx.notify();
}

pub(crate) fn open_sftp_confirm_modal(
    shell: &mut AppShell,
    action: SftpPendingDelete,
    title: String,
    description: String,
    cx: &mut Context<AppShell>,
) {
    let modal = cx.new(|_| SftpConfirmModalView::new(action, title, description));
    let sub = cx.subscribe(&modal, |shell, view, event: &SftpConfirmModalEvent, cx| match event {
        SftpConfirmModalEvent::Confirm => {
            let action = view.read(cx).action.clone();
            close_sftp_confirm_modal(shell, cx);
            crate::sftp::execute_sftp_delete(shell, action, cx);
        }
        SftpConfirmModalEvent::Cancel => close_sftp_confirm_modal(shell, cx),
    });
    shell.state.sftp_modals.confirm = Some(modal);
    shell.state.sftp_modals.subs.confirm = Some(sub);
    cx.notify();
}

pub(crate) fn open_sftp_properties_modal(
    shell: &mut AppShell,
    scope: String,
    properties: FileProperties,
    cx: &mut Context<AppShell>,
) {
    let modal = cx.new(|_| SftpPropertiesModalView::new(scope, properties));
    let sub = cx.subscribe(&modal, |shell, _, event: &SftpPropertiesModalEvent, cx| match event {
        SftpPropertiesModalEvent::Close => close_sftp_properties_modal(shell, cx),
    });
    shell.state.sftp_modals.properties = Some(modal);
    shell.state.sftp_modals.subs.properties = Some(sub);
    cx.notify();
}

pub(crate) fn open_sftp_filter_modal(
    shell: &mut AppShell,
    textarea: Entity<TextareaState>,
    cx: &mut Context<AppShell>,
) {
    let modal = cx.new(|_| SftpFilterModalView::new(textarea));
    let sub = cx.subscribe(&modal, |shell, _, event: &SftpFilterModalEvent, cx| match event {
        SftpFilterModalEvent::Save(text) => {
            update_app_settings(cx, |s| s.sftp.transfer_filter = text.clone());
            let settings_snapshot = app_settings(cx).clone();
            match shell.state.persist_settings(&settings_snapshot) {
                Ok(()) => {
                    shell.state.status = Some(rust_i18n::t!("sftp.filter_rules_saved").to_string());
                }
                Err(error) => {
                    shell.state.set_error(localized_error(&error));
                }
            }
            close_sftp_filter_modal(shell, cx);
        }
        SftpFilterModalEvent::Cancel => close_sftp_filter_modal(shell, cx),
    });
    shell.state.sftp_modals.filter = Some(modal);
    shell.state.sftp_modals.subs.filter = Some(sub);
    cx.notify();
}

pub(crate) fn open_sftp_chmod_modal(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: Option<ConnectionId>,
    file_name: String,
    remote_path: String,
    permissions: u32,
    octal_input: Entity<InputState>,
    cx: &mut Context<AppShell>,
) {
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
    let sub = cx.subscribe(&modal, |shell, _, event: &SftpChmodModalEvent, cx| match event {
        SftpChmodModalEvent::Apply {
            tab_id,
            connection_id,
            remote_path,
            permissions,
        } => {
            close_sftp_chmod_modal(shell, cx);
            let Some(conn_id) = connection_id.or_else(|| {
                shell
                    .state
                    .workspace
                    .tab(*tab_id)
                    .and_then(|tab| tab.connection_id)
            }) else {
                return;
            };
            crate::sftp::remote_actions::send_sftp_op_for_connection(
                shell,
                *tab_id,
                conn_id,
                SftpOperation::Chmod {
                    path: remote_path.clone(),
                    permissions: *permissions,
                },
                cx,
            );
        }
        SftpChmodModalEvent::Cancel => close_sftp_chmod_modal(shell, cx),
    });
    shell.state.sftp_modals.chmod = Some(modal);
    shell.state.sftp_modals.subs.chmod = Some(sub);
    cx.notify();
}

pub(crate) fn subscribe_sftp_input_modal(
    shell: &mut AppShell,
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
                    shell,
                    *tab_id,
                    *connection_id,
                    target,
                    value,
                    window,
                    cx,
                ) {
                    view.update(cx, |view, cx| view.set_error(err, window, cx));
                    return;
                }
                close_sftp_input_modal(shell, cx);
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
            SftpInputModalEvent::Cancel => {
                close_sftp_input_modal(shell, cx);
                crate::sftp::focus_active_sftp_panel(shell, window, cx);
            }
        },
    );
    shell.state.sftp_modals.input = Some(modal);
    shell.state.sftp_modals.subs.input = Some(sub);
    cx.notify();
}

pub(crate) fn open_sftp_sync_modal(
    shell: &mut AppShell,
    state: SftpSyncModalState,
    cx: &mut Context<AppShell>,
) {
    let modal = cx.new(|_| SftpSyncModalView::new(state));
    let sub = cx.subscribe(&modal, |shell, _, event: &SftpSyncModalEvent, cx| match event {
        SftpSyncModalEvent::Rescan(tab_id) => {
            crate::sftp::sftp_open_sync_modal(shell, *tab_id, cx);
        }
        SftpSyncModalEvent::Start {
            tab_id,
            local_root,
            remote_root,
            entries,
        } => {
            let connection_id = shell
                .state
                .sftp_modals
                .sync
                .as_ref()
                .map(|m| m.read(cx).state.connection_id);
            let started = crate::sftp::execute_sftp_sync(
                shell,
                *tab_id,
                connection_id,
                local_root,
                remote_root,
                entries,
                cx,
            );
            if started {
                close_sftp_sync_modal(shell, cx);
            }
        }
        SftpSyncModalEvent::Cancel => close_sftp_sync_modal(shell, cx),
    });
    shell.state.sftp_modals.sync = Some(modal);
    shell.state.sftp_modals.subs.sync = Some(sub);
    cx.notify();
}

pub(crate) fn open_sftp_search_modal(
    shell: &mut AppShell,
    state: SftpSearchModalState,
    cx: &mut Context<AppShell>,
) {
    let modal = cx.new(|_| SftpSearchModalView::new(state));
    let sub = cx.subscribe(&modal, |shell, _, event: &SftpSearchModalEvent, cx| match event {
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
                    .and_then(|tab| tab.connection_id)
            }) else {
                return;
            };
            crate::sftp::remote_actions::send_sftp_op_for_connection(
                shell,
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
            close_sftp_search_modal(shell, cx);
            if let Some(conn_id) = connection_id {
                crate::sftp::request_sftp_list_for_connection(
                    shell,
                    *tab_id,
                    *conn_id,
                    dir.clone(),
                    cx,
                );
            } else {
                crate::sftp::request_sftp_list(shell, *tab_id, dir.clone(), cx);
            }
        }
        SftpSearchModalEvent::Cancel => close_sftp_search_modal(shell, cx),
    });
    shell.state.sftp_modals.search = Some(modal);
    shell.state.sftp_modals.subs.search = Some(sub);
    cx.notify();
}
