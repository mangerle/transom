//! SFTP 模态输入框提交处理与本地软链接创建。

use crate::app::spawn_blocking_local;
use crate::app::{AppShell, AppState};
use crate::sftp::{SftpInputTarget, SftpRemotePaneRef, join_remote, parent_remote};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{ConnectionId, SftpOperation, TabId};

/// SFTP 输入弹窗提交一次的参数集合，避免七个平铺参数造成顺序错误。
pub(in crate::app::sftp) struct SftpInputCommit<'a> {
    pub tab_id: TabId,
    pub connection_id: Option<ConnectionId>,
    pub target: &'a SftpInputTarget,
    pub name: &'a str,
}

/// 执行已通过基础名称校验的 SFTP 输入弹窗提交动作。
pub(in crate::app::sftp) fn commit_sftp_input_value(
    app_state: &mut AppState,
    params: SftpInputCommit<'_>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> Result<(), String> {
    let SftpInputCommit {
        tab_id,
        connection_id,
        target: modal_target,
        name,
    } = params;
    let pane = match connection_id {
        Some(connection_id) => SftpRemotePaneRef::Side(connection_id),
        None => SftpRemotePaneRef::Main,
    };
    let fallback_main = app_state
        .workspace
        .tab(tab_id)
        .and_then(|tab| tab.connection_id());
    let scope = crate::sftp::sftp_state(&app_state.workspace, tab_id, cx)
        .and_then(|panel| panel.remote_pane_scope(pane, fallback_main))
        .map(|scope| SftpInputScope {
            connection_id: scope.connection_id,
            base_path: scope.pane.path.clone(),
            entries: scope.pane.entries.clone(),
        });

    if matches!(modal_target, SftpInputTarget::RemoteEditorNewFile) {
        if let Some(scope) = scope {
            return commit_new_remote_file(
                app_state,
                NewRemoteFileParams {
                    tab_id,
                    name,
                    scope: &scope,
                },
                window,
                cx,
            );
        }
        return Ok(());
    }

    match modal_target {
        SftpInputTarget::SudoElevation { editor } => {
            let password = (!name.trim().is_empty()).then(|| name.to_string());
            app_state.features.editors.update(cx, |panels, cx| {
                panels.submit_sudo_editor_write(editor, password, cx);
            });
        }
        SftpInputTarget::RemoteMkdir
        | SftpInputTarget::RemoteRename { .. }
        | SftpInputTarget::RemoteSymlink { .. }
        | SftpInputTarget::RemoteArchive { .. }
        | SftpInputTarget::RemoteExtract { .. } => {
            if let Some(scope) = &scope
                && let Some(op) = remote_operation(modal_target, &scope.base_path, name)
            {
                crate::sftp::send_sftp_op_for_connection(
                    app_state,
                    tab_id,
                    scope.connection_id,
                    op,
                    cx,
                );
            }
        }
        SftpInputTarget::RemoteEditorNewFile => {}
        SftpInputTarget::LocalMkdir
        | SftpInputTarget::LocalNewFile
        | SftpInputTarget::LocalRename { .. }
        | SftpInputTarget::LocalSymlink { .. } => {
            commit_local_input(app_state, modal_target, name, tab_id, cx);
        }
    }
    Ok(())
}

/// 远端新建文件的参数集合，避免六个平铺参数。
struct NewRemoteFileParams<'a> {
    tab_id: TabId,
    name: &'a str,
    scope: &'a SftpInputScope,
}

/// 远端新建文件：同名目录报错，同名文件直接打开，全新路径生成空编辑面板。
fn commit_new_remote_file(
    app_state: &mut AppState,
    params: NewRemoteFileParams<'_>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> Result<(), String> {
    let NewRemoteFileParams {
        tab_id,
        name,
        scope,
    } = params;
    let path = join_remote(&scope.base_path, name);
    let existing_is_dir = scope
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .map(|entry| entry.is_dir);
    match existing_is_dir {
        Some(true) => {
            Err(rust_i18n::t!("sftp.file_ops.file_exists_as_dir", name = name).to_string())
        }
        Some(false) => {
            crate::editor::sftp_edit_remote_file_on(
                app_state,
                tab_id,
                scope.connection_id,
                path,
                window,
                cx,
            );
            Ok(())
        }
        None => {
            crate::editor::spawn_new_editor_panel(
                app_state,
                tab_id,
                scope.connection_id,
                path,
                name.to_string(),
                window,
                cx,
            );
            Ok(())
        }
    }
}

/// 本地目标：新建目录/文件、重命名或软链接。
fn commit_local_input(
    app_state: &mut AppState,
    target: &SftpInputTarget,
    name: &str,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    let Some(local_path) = crate::sftp::sftp_state(&app_state.workspace, tab_id, cx)
        .map(|panel| panel.local.path.clone())
    else {
        return;
    };
    if let SftpInputTarget::LocalRename { old_path } = target {
        let Some(parent) = old_path.parent() else {
            if let Some(mut panel) = crate::sftp::sftp_view_mut(&app_state.workspace, tab_id, cx) {
                panel.state.set_local_error(
                    rust_i18n::t!(
                        "sftp.file_ops.local_rename_failed",
                        error = rust_i18n::t!("sftp.file_ops.no_parent_dir")
                    )
                    .to_string(),
                );
            }
            cx.notify();
            return;
        };
        spawn_local_fs(
            tab_id,
            LocalFsOp::Rename(old_path.clone(), parent.join(name)),
            cx,
        );
        return;
    }
    let op = match target {
        SftpInputTarget::LocalMkdir => LocalFsOp::Mkdir(local_path.join(name)),
        SftpInputTarget::LocalNewFile => LocalFsOp::NewFile(local_path.join(name)),
        SftpInputTarget::LocalSymlink { target_path } => {
            LocalFsOp::Symlink(target_path.clone(), local_path.join(name))
        }
        _ => return,
    };
    spawn_local_fs(tab_id, op, cx);
}

/// 本次输入的目标远端范围：连接、当前目录与条目快照。
struct SftpInputScope {
    connection_id: transom_core::ConnectionId,
    base_path: String,
    entries: Vec<transom_core::RemoteFile>,
}

/// 由输入目标构造远端操作；本地目标返回 `None`。
fn remote_operation(
    target: &SftpInputTarget,
    base_path: &str,
    name: &str,
) -> Option<SftpOperation> {
    match target {
        SftpInputTarget::RemoteMkdir => Some(SftpOperation::Mkdir {
            path: join_remote(base_path, name),
        }),
        SftpInputTarget::RemoteRename { old_path } => Some(SftpOperation::Rename {
            from: old_path.clone(),
            to: join_remote(&parent_remote(old_path), name),
        }),
        SftpInputTarget::RemoteSymlink { target } => Some(SftpOperation::Symlink {
            target: target.clone(),
            link_path: join_remote(base_path, name),
        }),
        SftpInputTarget::RemoteArchive {
            source_paths,
            parent_dir,
        } => Some(SftpOperation::Archive {
            source_paths: source_paths.clone(),
            dest_archive_path: join_remote(parent_dir, name),
        }),
        SftpInputTarget::RemoteExtract {
            archive_path,
            parent_dir,
        } => Some(SftpOperation::Extract {
            archive_path: archive_path.clone(),
            dest_dir: join_remote(parent_dir, name),
        }),
        _ => None,
    }
}

/// 本地文件操作：在后台线程执行，成功后回到 UI 线程刷新。
enum LocalFsOp {
    Mkdir(PathBuf),
    NewFile(PathBuf),
    Rename(PathBuf, PathBuf),
    Symlink(PathBuf, PathBuf),
}

/// 本地文件操作结果；字段都是已本地化文案或待选中路径。
struct LocalFsOutcome {
    status: Option<String>,
    error: Option<String>,
    select: Option<PathBuf>,
    reveal: Option<PathBuf>,
}

fn run_local_fs(op: LocalFsOp) -> LocalFsOutcome {
    match op {
        LocalFsOp::Mkdir(path) => match std::fs::create_dir_all(&path) {
            Ok(()) => LocalFsOutcome {
                status: Some(
                    rust_i18n::t!(
                        "sftp.file_ops.local_mkdir_created",
                        name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned()
                        )
                    )
                    .to_string(),
                ),
                error: None,
                select: None,
                reveal: None,
            },
            Err(error) => LocalFsOutcome {
                status: None,
                error: Some(
                    rust_i18n::t!("sftp.file_ops.local_mkdir_failed", error = error).to_string(),
                ),
                select: None,
                reveal: None,
            },
        },
        LocalFsOp::NewFile(path) => {
            if path.exists() {
                return LocalFsOutcome {
                    status: None,
                    error: Some(
                        rust_i18n::t!("sftp.file_ops.local_file_exists", name = path.display())
                            .to_string(),
                    ),
                    select: None,
                    reveal: None,
                };
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => LocalFsOutcome {
                    status: Some(
                        rust_i18n::t!("sftp.file_ops.local_file_created", name = path.display())
                            .to_string(),
                    ),
                    error: None,
                    select: Some(path.clone()),
                    reveal: Some(path),
                },
                Err(error) => LocalFsOutcome {
                    status: None,
                    error: Some(
                        rust_i18n::t!("sftp.file_ops.local_file_create_failed", error = error)
                            .to_string(),
                    ),
                    select: None,
                    reveal: None,
                },
            }
        }
        LocalFsOp::Rename(from, to) => match std::fs::rename(&from, &to) {
            Ok(()) => LocalFsOutcome {
                status: Some(
                    rust_i18n::t!("sftp.file_ops.local_renamed", name = to.display()).to_string(),
                ),
                error: None,
                select: Some(to),
                reveal: None,
            },
            Err(error) => LocalFsOutcome {
                status: None,
                error: Some(
                    rust_i18n::t!("sftp.file_ops.local_rename_failed", error = error).to_string(),
                ),
                select: None,
                reveal: None,
            },
        },
        LocalFsOp::Symlink(target, link) => {
            let result = transom_platform::filesystem::create_symlink(&target, &link)
                .map_err(|error| error.to_string());
            match result {
                Ok(()) => LocalFsOutcome {
                    status: Some(
                        rust_i18n::t!("sftp.file_ops.local_symlink_created", name = link.display())
                            .to_string(),
                    ),
                    error: None,
                    select: Some(link),
                    reveal: None,
                },
                Err(error) => LocalFsOutcome {
                    status: None,
                    error: Some(
                        rust_i18n::t!("sftp.file_ops.local_symlink_failed", error = error)
                            .to_string(),
                    ),
                    select: None,
                    reveal: None,
                },
            }
        }
    }
}

/// 在后台线程执行本地文件操作，完成后回 UI 刷新对应面板。
fn spawn_local_fs(tab_id: TabId, op: LocalFsOp, cx: &mut Context<AppShell>) {
    cx.spawn(async move |this, cx| {
        let Some(outcome) = spawn_blocking_local(move || run_local_fs(op)).await else {
            log::error!("本地文件操作线程创建失败");
            return;
        };
        let _ = this.update(cx, |this, cx| {
            apply_local_fs_outcome(&mut this.state, tab_id, outcome, cx);
        });
    })
    .detach();
}

/// 应用本地文件操作结果：错误与状态写回面板，成功时刷新并选中目标。
fn apply_local_fs_outcome(
    app_state: &mut AppState,
    tab_id: TabId,
    outcome: LocalFsOutcome,
    cx: &mut Context<AppShell>,
) {
    let (select, reveal) = {
        let Some(mut panel) = crate::sftp::sftp_view_mut(&app_state.workspace, tab_id, cx) else {
            return;
        };
        if let Some(error) = outcome.error {
            panel.state.set_local_error(error);
        }
        if let Some(status) = outcome.status {
            panel.state.set_status(status);
        }
        (outcome.select, outcome.reveal)
    };
    if let Some(path) = select
        && let Some(view) = app_state.sftp_view(tab_id)
    {
        view.update(cx, |view, cx| view.spawn_local_dir_refresh(Some(path), cx));
    }
    if let Some(path) = reveal {
        crate::audit::reveal_in_file_manager(path);
    }
    cx.notify();
}
