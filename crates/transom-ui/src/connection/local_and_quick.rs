//! 本地终端启动、快捷连接、桌面深链与命令历史记录。

use crate::app::app_settings;
use crate::app::{AppShell, TerminalTab};
use crate::keymap;
use gpui_kit::*;
use transom_core::{DeepLinkRequest, LocalConnection, Session, TabId, parse_ssh_config};
use transom_platform::shell::ShellOption;

pub(crate) fn on_new_local_terminal(
    shell: &mut AppShell,
    _: &keymap::NewLocalTerminal,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::connection::open_local_terminal(shell, window, cx);
}

pub(crate) fn on_quick_open(
    shell: &mut AppShell,
    _: &keymap::QuickOpen,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::connection::toggle_quick_open(shell, window, cx);
}

pub(crate) fn toggle_quick_open(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    shell.state.ui.modals.tools.show_quick_open = !shell.state.ui.modals.tools.show_quick_open;
    if shell.state.ui.modals.tools.show_quick_open {
        shell
            .state
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
    shell: &mut AppShell,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::connection::spawn_local_command(shell, None, None, cx);
}

/// 新开一个本地终端标签。
pub(crate) fn spawn_local_terminal(
    shell: &mut AppShell,
    working_dir: Option<std::path::PathBuf>,
    cx: &mut Context<AppShell>,
) {
    crate::connection::spawn_local_command(shell, None, working_dir, cx);
}

/// 新开一个本地终端标签，可选地在其中直接执行命令（`X-TerminalArgExec`）。
///
/// 命令按 argv 原样交给 PTY 层执行，不拼 Shell 字符串：launcher 已经完成
/// 分词，再拼一次会破坏含空格、引号与 `$` 的参数。标题取命令行的前一段，
/// 让用户一眼看出这个标签不是普通 Shell。
pub(crate) fn spawn_local_command(
    shell: &mut AppShell,
    command: Option<Vec<String>>,
    working_dir: Option<std::path::PathBuf>,
    cx: &mut Context<AppShell>,
) {
    spawn_local_request(shell, command, working_dir, None, cx);
}

/// 用用户本次明确选择的 Shell 新开标签；与默认设置分离，选择只影响本次。
pub(crate) fn spawn_local_shell(
    app: &mut AppShell,
    shell: ShellOption,
    run_as_admin: bool,
    cx: &mut Context<AppShell>,
) {
    spawn_local_request(app, None, None, Some((shell, run_as_admin)), cx);
}

fn spawn_local_request(
    app: &mut AppShell,
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
    app.state
        .workspace
        .create_local_group(tab_id, session.id, &session.name);
    app.state
        .workspace
        .tabs
        .push(TerminalTab::pending(tab_id, session.clone()));
    app.state.workspace.active_tab = Some(tab_id);
    app.state.present_active_group();
    crate::connection::start_connection(app, tab_id, session, cx);
}

/// Quick Connect：解析主机串并发起不落盘的临时 SSH 连接。
pub(crate) fn connect_quick_host(shell: &mut AppShell, host: &str, cx: &mut Context<AppShell>) {
    let session = match transom_core::quick_connect_session(host) {
        Ok(session) => session,
        Err(error) => {
            shell.state.set_error(error);
            cx.notify();
            return;
        }
    };
    let tab_id = TabId::new();
    shell.state.workspace.create_group(tab_id, session.id);
    shell
        .state
        .workspace
        .tabs
        .push(TerminalTab::pending(tab_id, session.clone()));
    shell.state.workspace.active_tab = Some(tab_id);
    shell.state.present_active_group();
    crate::connection::start_connection(shell, tab_id, session, cx);
}

/// 桌面深链唤起。
pub(crate) fn handle_deep_link(
    shell: &mut AppShell,
    request: DeepLinkRequest,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    cx.activate(true);
    window.activate_window();
    match request {
        DeepLinkRequest::Ssh { url } => crate::connection::connect_quick_host(shell, &url, cx),
        DeepLinkRequest::Dir { path } => {
            crate::connection::spawn_local_terminal(shell, Some(path), cx)
        }
        DeepLinkRequest::Exec { command, dir } => {
            crate::connection::spawn_local_command(shell, Some(command), dir, cx);
        }
        DeepLinkRequest::Activate => cx.notify(),
    }
}

/// 记录一条采集到的命令：更新内存快照并后台落盘。
pub(crate) fn record_history_command(
    shell: &mut AppShell,
    command: &str,
    cx: &mut Context<AppShell>,
) {
    if !app_settings(cx).history.enabled {
        return;
    }
    let recorded = {
        let mut snapshot = shell.state.runtime.history.write();
        transom_core::record_command(&mut snapshot, command)
    };
    if !recorded {
        return;
    }
    let snapshot: Vec<String> = shell.state.runtime.history.read().clone();
    let store = shell.state.runtime.services.history.clone();
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
pub(crate) fn import_ssh_config(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let ssh_config_path = transom_platform::paths::ssh_config_path();

    let Some(path) = ssh_config_path else {
        shell
            .state
            .set_error(rust_i18n::t!("app.import_none").to_string());
        cx.notify();
        return;
    };

    if !path.exists() {
        shell
            .state
            .set_error(rust_i18n::t!("app.import_none").to_string());
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
            Ok(content) => finish_import_ssh_config(this, content, cx),
            Err(error) => {
                this.state.set_error(error.to_string());
                cx.notify();
            }
        });
    })
    .detach();
}

/// 解析并合并 SSH 配置；在界面线程执行。
fn finish_import_ssh_config(shell: &mut AppShell, content: String, cx: &mut Context<AppShell>) {
    let parsed = parse_ssh_config(&content);
    if parsed.is_empty() {
        shell
            .state
            .set_error(rust_i18n::t!("app.import_none").to_string());
        cx.notify();
        return;
    }

    let folder = shell.state.session.catalog.ensure_folder_path("SSH Config");
    let mut added_count = 0;
    for mut session in parsed {
        if !shell
            .state
            .session
            .catalog
            .sessions
            .iter()
            .any(|s| s.name == session.name)
        {
            session.folder_id = folder;
            shell.state.session.catalog.sessions.push(session);
            added_count += 1;
        }
    }

    if added_count > 0 {
        if shell.state.persist_catalog() {
            shell.state.ui.status =
                Some(rust_i18n::t!("app.import_success", count = added_count).to_string());
        }
    } else {
        shell.state.ui.status = Some(rust_i18n::t!("app.import_none").to_string());
    }
    cx.notify();
}
