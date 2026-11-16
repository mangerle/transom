//! 远端编辑面板打开、新建与终端代码链接跳转的外壳桥接。

use super::editor_reject;
use crate::app::{AppShell, app_settings};
use crate::editor::code_link::resolve_code_path;
use crate::editor::navigate::apply_editor_goto;
use crate::editor::open::{EditGate, EditorPanelSettings, PanelSeed, SpawnEditorPanelParams};
use crate::sftp::format_size;
use gpui_kit::*;
use transom_core::{AppSettings, ConnectionId, TabId, TransferId};

/// 已通过目录闸门的远端编辑面板打开参数。
#[derive(Clone)]
pub(crate) struct RemoteEditorOpen {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    pub(crate) permissions: Option<u32>,
}

/// 终端代码链接转换出的远端定位请求（行列均为 1 基）。
pub(crate) struct RemoteCodeLocation {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_path: String,
    pub(crate) line: u32,
    pub(crate) column: Option<u32>,
}

pub(crate) fn sftp_edit_remote_file_on(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    remote_path: String,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let max_bytes = app_settings(cx).sftp.edit_max_bytes;
    let gate = shell
        .state
        .workspace
        .tab(tab_id)
        .and_then(|tab| {
            let panel = crate::sftp::sftp_state(&shell.state.workspace, tab_id, cx)?;
            let entries = if tab.connection_id == Some(connection_id) {
                &panel.remote.entries
            } else {
                &panel
                    .side_remotes
                    .iter()
                    .find(|side| side.connection_id == Some(connection_id))?
                    .entries
            };
            let entry = entries.iter().find(|item| item.path == remote_path)?;
            if entry.is_dir {
                return Some(EditGate::IsDir);
            }
            if entry.size > max_bytes {
                return Some(EditGate::TooLarge {
                    name: entry.name.clone(),
                    size: entry.size,
                });
            }
            Some(EditGate::Allowed {
                connection_id,
                name: entry.name.clone(),
                permissions: entry.permissions,
            })
        })
        .unwrap_or(EditGate::Missing);

    let (connection_id, file_name, permissions) = match gate {
        EditGate::Missing => {
            editor_reject(
                shell,
                tab_id,
                connection_id,
                rust_i18n::t!("sftp.select_file").to_string(),
                cx,
            );
            return;
        }
        EditGate::IsDir => return,
        EditGate::TooLarge { name, size } => {
            editor_reject(
                shell,
                tab_id,
                connection_id,
                rust_i18n::t!(
                    "editor.error_too_large",
                    name = name,
                    size = format_size(size),
                    max = format_size(max_bytes)
                )
                .to_string(),
                cx,
            );
            return;
        }
        EditGate::Allowed {
            connection_id,
            name,
            permissions,
        } => (connection_id, name, permissions),
    };

    open_remote_editor_panel(
        shell,
        RemoteEditorOpen {
            tab_id,
            connection_id,
            remote_path,
            file_name,
            permissions,
        },
        window,
        cx,
    );
}

/// 已通过目录闸门的远端文件：抬高已存在的编辑器或新建读取面板。
pub(crate) fn open_remote_editor_panel(
    shell: &mut AppShell,
    params: RemoteEditorOpen,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let RemoteEditorOpen {
        tab_id,
        connection_id,
        remote_path,
        file_name,
        permissions,
    } = params;
    if let Some(editor) = shell.state.features.editors.update(cx, |panels, _| {
        panels.raise_existing(tab_id, connection_id, &remote_path)
    }) {
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
        return;
    }

    let op_id = TransferId::new();
    let settings = editor_panel_settings(app_settings(cx));
    let request = shell.state.features.editors.update(cx, |panels, cx| {
        panels.spawn_editor_panel(
            SpawnEditorPanelParams {
                tab_id,
                connection_id,
                remote_path,
                file_name,
                seed: PanelSeed::Reading { op_id, permissions },
                pending_goto: None,
            },
            settings,
            window,
            cx,
        )
    });
    if let Some(request) = request {
        shell.state.features.editors.update(cx, |panels, cx| {
            panels.request_remote_text_read(request, cx);
        });
    }
}

/// 新建远端文件并打开编辑面板：空缓冲区立即可编辑，首次保存才创建远端文件。
pub(crate) fn spawn_new_editor_panel(
    shell: &mut AppShell,
    tab_id: TabId,
    connection_id: ConnectionId,
    remote_path: String,
    file_name: String,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let settings = editor_panel_settings(app_settings(cx));
    shell.state.features.editors.update(cx, |panels, cx| {
        panels.spawn_editor_panel(
            SpawnEditorPanelParams {
                tab_id,
                connection_id,
                remote_path,
                file_name,
                seed: PanelSeed::NewFile,
                pending_goto: None,
            },
            settings,
            window,
            cx,
        )
    });
}

/// 处理终端投递的代码位置链接。
pub(crate) fn open_terminal_code_location(
    shell: &mut AppShell,
    tab_id: TabId,
    path: &str,
    line: u32,
    column: Option<u32>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(connection_id) = shell
        .state
        .workspace
        .tab(tab_id)
        .map(|tab| tab.connection_id)
    else {
        return;
    };
    let remote_dir = crate::sftp::sftp_state(&shell.state.workspace, tab_id, cx)
        .map(|panel| panel.remote.path.clone());
    let Some(resolved) = resolve_code_path(path, remote_dir.as_deref()) else {
        shell.state.ui.status =
            Some(rust_i18n::t!("editor.error_unresolved_link", path = path).to_string());
        cx.notify();
        return;
    };
    let Some(connection_id) = connection_id else {
        let local = std::path::Path::new(&resolved);
        if local.is_absolute() {
            crate::audit::open_local_path(local);
        } else {
            shell.state.ui.status =
                Some(rust_i18n::t!("editor.error_local_absolute", path = resolved).to_string());
            cx.notify();
        }
        return;
    };
    open_remote_code_location(
        shell,
        RemoteCodeLocation {
            tab_id,
            connection_id,
            remote_path: resolved,
            line,
            column,
        },
        window,
        cx,
    );
}

/// 从终端 OSC 8 代码链接直接打开远端文件并定位到行列。
pub(crate) fn open_remote_code_location(
    shell: &mut AppShell,
    target: RemoteCodeLocation,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let RemoteCodeLocation {
        tab_id,
        connection_id,
        remote_path,
        line,
        column,
    } = target;
    if let Some(editor) = shell.state.features.editors.update(cx, |panels, _| {
        panels.raise_existing(tab_id, connection_id, &remote_path)
    }) {
        apply_editor_goto(&editor, line, column, window, cx);
        cx.notify();
        return;
    }
    let file_name = remote_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map_or_else(
            || rust_i18n::t!("editor.remote_file_fallback").to_string(),
            |name| name.to_string(),
        );
    let op_id = TransferId::new();
    let settings = editor_panel_settings(app_settings(cx));
    let request = shell.state.features.editors.update(cx, |panels, cx| {
        panels.spawn_editor_panel(
            SpawnEditorPanelParams {
                tab_id,
                connection_id,
                remote_path,
                file_name,
                seed: PanelSeed::Reading {
                    op_id,
                    permissions: None,
                },
                pending_goto: Some((line, column)),
            },
            settings,
            window,
            cx,
        )
    });
    if let Some(request) = request {
        shell.state.features.editors.update(cx, |panels, cx| {
            panels.request_remote_text_read(request, cx);
        });
    }
}

fn editor_panel_settings(settings: &AppSettings) -> EditorPanelSettings {
    EditorPanelSettings {
        line_numbers: settings.sftp.editor_line_numbers,
        soft_wrap: settings.sftp.editor_soft_wrap,
    }
}
