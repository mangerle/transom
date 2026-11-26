//! 本地终端启动、快捷连接、桌面深链与命令历史记录。

use crate::app::app_settings;
use crate::app::{AppShell, AppState, WorkspaceTab};
use crate::keymap;
use crate::session_form::ExternalSessionImportKind;
use gpui_kit::*;
use transom_core::{
    DeepLinkRequest, ImportedSessionEntry, LocalConnection, Session, TabId,
    parse_mobaxterm_sessions, parse_ssh_config, parse_xshell_file, scan_xshell_directory,
};
use transom_platform::shell::ShellOption;

pub(crate) fn on_new_local_terminal(
    app_state: &mut AppState,
    _: &keymap::NewLocalTerminal,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::connection::open_local_terminal(app_state, window, cx);
}

pub(crate) fn on_quick_open(
    app_state: &mut AppState,
    _: &keymap::QuickOpen,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::connection::toggle_quick_open(app_state, window, cx);
}

pub(crate) fn toggle_quick_open(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    app_state.ui.modals.tools.show_quick_open = !app_state.ui.modals.tools.show_quick_open;
    if app_state.ui.modals.tools.show_quick_open {
        app_state
            .ui
            .modals
            .tools
            .quick_open_state
            .update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
    }
    cx.notify();
}

pub(crate) fn open_local_terminal(
    app_state: &mut AppState,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::connection::spawn_local_command(app_state, None, None, cx);
}

/// 新开一个本地终端标签。
pub(crate) fn spawn_local_terminal(
    app_state: &mut AppState,
    working_dir: Option<std::path::PathBuf>,
    cx: &mut Context<AppShell>,
) {
    crate::connection::spawn_local_command(app_state, None, working_dir, cx);
}

/// 新开一个本地终端标签，可选地在其中直接执行命令（`X-TerminalArgExec`）。
///
/// 命令按 argv 原样交给 PTY 层执行，不拼 Shell 字符串：launcher 已经完成
/// 分词，再拼一次会破坏含空格、引号与 `$` 的参数。标题取命令行的前一段，
/// 让用户一眼看出这个标签不是普通 Shell。
pub(crate) fn spawn_local_command(
    app_state: &mut AppState,
    command: Option<Vec<String>>,
    working_dir: Option<std::path::PathBuf>,
    cx: &mut Context<AppShell>,
) {
    spawn_local_request(app_state, command, working_dir, None, cx);
}

/// 用用户本次明确选择的 Shell 新开标签；与默认设置分离，选择只影响本次。
pub(crate) fn spawn_local_shell(
    app_state: &mut AppState,
    shell: ShellOption,
    run_as_admin: bool,
    cx: &mut Context<AppShell>,
) {
    spawn_local_request(app_state, None, None, Some((shell, run_as_admin)), cx);
}

fn spawn_local_request(
    app_state: &mut AppState,
    command: Option<Vec<String>>,
    working_dir: Option<std::path::PathBuf>,
    shell_override: Option<(ShellOption, bool)>,
    cx: &mut Context<AppShell>,
) {
    let settings = app_settings(cx);
    let (shell, shell_args, run_as_admin, shell_label) = match shell_override {
        Some((shell, run_as_admin)) => (
            Some(shell.program),
            shell.args,
            run_as_admin,
            Some(shell.label),
        ),
        None => (
            settings.local_shell.clone(),
            settings.local_shell_args.clone(),
            cfg!(windows) && settings.local_terminal_run_as_admin && command.is_none(),
            None,
        ),
    };
    let mut title = match (
        &command,
        working_dir.as_deref().and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }),
    ) {
        (Some(argv), _) => {
            let line = argv.join(" ");
            let mut short: String = line.chars().take(40).collect();
            if line.chars().count() > 40 {
                short.push('…');
            }
            format!("{} · {short}", rust_i18n::t!("app.new_local"))
        }
        (None, Some(name)) => format!("{} · {name}", rust_i18n::t!("app.new_local")),
        (None, None) => match shell_label {
            Some(label) => format!("{} · {label}", rust_i18n::t!("app.new_local")),
            None => rust_i18n::t!("app.new_local").to_string(),
        },
    };
    if run_as_admin {
        title.push_str(" · ");
        title.push_str(&rust_i18n::t!("app.local_shell_admin_badge"));
    }
    let tab_id = TabId::new();
    let session = Session::new(
        title,
        transom_core::Connection::Local(LocalConnection {
            shell,
            shell_args,
            run_as_admin,
            working_dir,
            command,
            ..Default::default()
        }),
    );
    app_state
        .workspace
        .create_local_group(tab_id, session.id, &session.name);
    app_state
        .workspace
        .tabs
        .push(WorkspaceTab::new_session(tab_id, session.clone()));
    app_state.workspace.active_tab = Some(tab_id);
    app_state.present_active_group();
    crate::connection::start_connection(app_state, tab_id, session, cx);
}

/// Quick Connect：解析主机串并发起不落盘的临时 SSH 连接。
pub(crate) fn connect_quick_host(app_state: &mut AppState, host: &str, cx: &mut Context<AppShell>) {
    let session = match transom_core::quick_connect_session(host) {
        Ok(session) => session,
        Err(error) => {
            app_state.set_error(error);
            cx.notify();
            return;
        }
    };
    let tab_id = TabId::new();
    let tab = WorkspaceTab::new_session(tab_id, session.clone());
    app_state.workspace.create_group_for_tab(&tab);
    app_state.workspace.tabs.push(tab);
    app_state.workspace.active_tab = Some(tab_id);
    app_state.present_active_group();
    crate::connection::start_connection(app_state, tab_id, session, cx);
}

/// 桌面深链唤起。
pub(crate) fn handle_deep_link(
    app_state: &mut AppState,
    request: DeepLinkRequest,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    cx.activate(true);
    window.activate_window();
    match request {
        DeepLinkRequest::Ssh { url } => crate::connection::connect_quick_host(app_state, &url, cx),
        DeepLinkRequest::Dir { path } => {
            crate::connection::spawn_local_terminal(app_state, Some(path), cx)
        }
        DeepLinkRequest::Exec { command, dir } => {
            crate::connection::spawn_local_command(app_state, Some(command), dir, cx);
        }
        DeepLinkRequest::Activate => cx.notify(),
    }
}

/// 记录一条采集到的命令：更新内存快照并后台落盘。
pub(crate) fn record_history_command(
    app_state: &mut AppState,
    command: &str,
    cx: &mut Context<AppShell>,
) {
    if !app_settings(cx).history.enabled {
        return;
    }
    let recorded = {
        let mut snapshot = app_state.runtime.history.write();
        transom_core::record_command(&mut snapshot, command)
    };
    if !recorded {
        return;
    }
    let snapshot: Vec<String> = app_state.runtime.history.read().clone();
    let store = app_state.runtime.services.history.clone();
    cx.spawn(async move |_, cx| {
        let error = cx
            .background_executor()
            .spawn(async move { store.save(&snapshot).err() })
            .await;
        if let Some(error) = error {
            log::warn!("命令历史写盘失败，本次仍保留在内存中，原因: {error}");
        }
    })
    .detach();
}

/// 导入本地 ~/.ssh/config 配置。
///
/// 读取放在阻塞线程池：家目录位于 NFS 或配置较大时，同步读会卡住 UI
/// 线程；解析与目录合并仍在界面线程完成，保证状态只有一个写入点。
pub(crate) fn import_ssh_config(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    let ssh_config_path = transom_platform::paths::ssh_config_path();

    let Some(path) = ssh_config_path else {
        app_state.set_error(rust_i18n::t!("app.import_none").to_string());
        cx.notify();
        return;
    };

    if !path.exists() {
        app_state.set_error(rust_i18n::t!("app.import_none").to_string());
        cx.notify();
        return;
    }

    cx.spawn(async move |this, cx| {
        let Some(result) =
            crate::app::spawn_blocking_local(move || std::fs::read_to_string(path)).await
        else {
            log::error!("读取 SSH 配置的阻塞线程创建失败");
            return;
        };
        let _ = this.update(cx, |this, cx| match result {
            Ok(content) => finish_import_ssh_config(&mut this.state, content, cx),
            Err(error) => {
                this.state.set_error(error.to_string());
                cx.notify();
            }
        });
    })
    .detach();
}

/// 解析并合并 SSH 配置；在界面线程执行。
fn finish_import_ssh_config(app_state: &mut AppState, content: String, cx: &mut Context<AppShell>) {
    let parsed = parse_ssh_config(&content);
    if parsed.is_empty() {
        app_state.set_error(rust_i18n::t!("app.import_none").to_string());
        cx.notify();
        return;
    }

    let folder = app_state.session.catalog.ensure_folder_path("SSH Config");
    let mut added_count = 0;
    for mut session in parsed {
        if !app_state
            .session
            .catalog
            .sessions
            .iter()
            .any(|s| s.name == session.name)
        {
            session.folder_id = folder;
            app_state.session.catalog.sessions.push(session);
            added_count += 1;
        }
    }

    if added_count > 0 {
        if app_state.persist_catalog() {
            app_state.ui.status =
                Some(rust_i18n::t!("app.import_success", count = added_count).to_string());
        }
    } else {
        app_state.ui.status = Some(rust_i18n::t!("app.import_none").to_string());
    }
    cx.notify();
}

/// 从新建会话表单触发外部会话批量导入（Xshell `.xsh` / MobaXterm `.mxtsessions` / SSH 配置）。
///
/// 文件选择由 GPUI `prompt_for_paths` 异步拉起，磁盘读取与解析均在阻塞线程池
/// 完成，避免阻塞界面渲染线程。
pub(crate) fn import_external_sessions(
    app_state: &mut AppState,
    kind: ExternalSessionImportKind,
    cx: &mut Context<AppShell>,
) {
    if kind == ExternalSessionImportKind::SshConfig {
        import_ssh_config(app_state, cx);
        return;
    }

    let options = match kind {
        ExternalSessionImportKind::XshellFiles => PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        },
        ExternalSessionImportKind::XshellFolder => PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        },
        ExternalSessionImportKind::MobaXtermFile => PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        },
        ExternalSessionImportKind::SshConfig => unreachable!(),
    };

    let picked = cx.prompt_for_paths(options);
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let Some(entries) =
            crate::app::spawn_blocking_local(move || load_external_entries(kind, &paths)).await
        else {
            log::error!("读取外部会话文件的阻塞线程创建失败");
            return;
        };
        let _ = this.update(cx, |this, cx| match entries {
            Ok((root_folder, parsed)) => {
                finish_import_external_entries(&mut this.state, root_folder, parsed, cx);
            }
            Err(error) => {
                this.state.set_error(error);
                cx.notify();
            }
        });
    })
    .detach();
}

/// 在阻塞线程中读取并解析外部会话文件。
fn load_external_entries(
    kind: ExternalSessionImportKind,
    paths: &[std::path::PathBuf],
) -> Result<(&'static str, Vec<ImportedSessionEntry>), String> {
    match kind {
        ExternalSessionImportKind::XshellFiles => {
            let mut list = Vec::new();
            for path in paths {
                if let Some(entry) = parse_xshell_file(path, None) {
                    list.push(entry);
                }
            }
            Ok(("Xshell", list))
        }
        ExternalSessionImportKind::XshellFolder => {
            let Some(root) = paths.first() else {
                return Ok(("Xshell", Vec::new()));
            };
            Ok(("Xshell", scan_xshell_directory(root)))
        }
        ExternalSessionImportKind::MobaXtermFile => {
            let Some(path) = paths.first() else {
                return Ok(("MobaXterm", Vec::new()));
            };
            let content = std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            Ok(("MobaXterm", parse_mobaxterm_sessions(&content)))
        }
        ExternalSessionImportKind::SshConfig => Ok(("SSH Config", Vec::new())),
    }
}

/// 将外部导入的会话条目合并到目录（保留子文件夹层级并按目标文件夹内名称去重）。
fn finish_import_external_entries(
    app_state: &mut AppState,
    root_folder: &str,
    entries: Vec<ImportedSessionEntry>,
    cx: &mut Context<AppShell>,
) {
    if entries.is_empty() {
        app_state.set_error(rust_i18n::t!("form.import_external_none").to_string());
        cx.notify();
        return;
    }

    let mut added_count = 0usize;
    for mut entry in entries {
        let full_path = match entry.folder_path.as_deref() {
            Some(sub) if !sub.is_empty() => format!("{root_folder}/{sub}"),
            _ => root_folder.to_string(),
        };
        let folder_id = app_state.session.catalog.ensure_folder_path(&full_path);
        let duplicate = app_state
            .session
            .catalog
            .sessions
            .iter()
            .any(|s| s.folder_id == folder_id && s.name == entry.session.name);
        if !duplicate {
            entry.session.folder_id = folder_id;
            app_state.session.catalog.sessions.push(entry.session);
            added_count += 1;
        }
    }

    if added_count > 0 {
        if app_state.persist_catalog() {
            app_state.ui.status = Some(
                rust_i18n::t!(
                    "form.import_external_success",
                    count = added_count,
                    source = root_folder
                )
                .to_string(),
            );
        }
    } else {
        app_state.ui.status = Some(rust_i18n::t!("form.import_external_none").to_string());
    }
    cx.notify();
}
