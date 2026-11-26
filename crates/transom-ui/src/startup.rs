//! 连接就绪后的初始化命令调度。
//!
//! 命令必须写入交互式 Shell 才能保留 `cd`、环境变量与提权状态，因此这里
//! 通过 `TerminalView::send_text` 注入，而不是复用独立的 exec 通道。
//! 远端 PAM/MOTD 输出可能晚于连接握手完成，发送前保留一个短暂的稳定窗口；
//! 每条命令之间也留出间隔，降低多行粘贴被 Shell 合并解析的概率。

use crate::app::{AppShell, AppState};
use gpui_kit::*;
use std::time::Duration;
use transom_core::{Connection, ConnectionId, Session, TabId};

/// 连接建立到第一条命令之间的等待时间。
const STARTUP_GRACE: Duration = Duration::from_millis(700);
/// 相邻命令之间的等待时间。
const STARTUP_INTERVAL: Duration = Duration::from_millis(250);

/// 把 POSIX 路径用单引号安全转义，避免空格或特殊字符破坏 `cd` 命令。
fn quote_posix_arg(raw: &str) -> String {
    let escaped = raw.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// 根据 SSH 会话持久化与目录恢复选项生成连接就绪后的附加命令。
///
/// 设计原理（Why）：开启 `persist_tmux_session` 时，tmux 会自行保留会话内的工作目录与
/// 前台进程，因此优先接入 tmux 而不再额外注入 `cd`；仅在未启用 tmux 且存在断线前的
/// `last_cwd` 时才下发 `cd` 切回原目录。
pub(crate) fn build_persistence_commands(session: &Session, last_cwd: Option<&str>) -> Vec<String> {
    let Connection::Ssh(ssh) = &session.connection else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    if ssh.options.persist_tmux_session {
        let short_id = session.id.to_string().chars().take(8).collect::<String>();
        let tmux_name = format!("transom_{short_id}");
        commands.push(format!(
            "command -v tmux >/dev/null 2>&1 && tmux new-session -A -s {tmux_name}"
        ));
        return commands;
    }
    if ssh.options.restore_cwd_on_reconnect
        && let Some(cwd) = last_cwd.filter(|path| !path.trim().is_empty())
    {
        commands.push(format!("cd -- {}", quote_posix_arg(cwd)));
    }
    commands
}

/// 为已就绪的终端安排初始化命令。
///
/// 任务在标签关闭、重连或连接切换时通过 `_startup_task` 句柄取消；
/// 每轮写入前都会校验 `connection_id`，避免旧连接的命令误写到新连接。
pub(crate) fn schedule_startup_commands(
    app_state: &mut AppState,
    tab_id: TabId,
    connection_id: ConnectionId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(commands) = app_state
        .workspace
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id && tab.connection_id() == Some(connection_id))
        .and_then(|tab| {
            let session = tab.session()?;
            let last_cwd = tab.session_pane().and_then(|pane| pane.last_cwd.as_deref());
            let mut commands = session.startup.commands.clone();
            commands.extend(build_persistence_commands(session, last_cwd));
            Some(commands)
        })
        .filter(|commands| !commands.is_empty())
    else {
        return;
    };

    let task = cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(STARTUP_GRACE).await;
        for command in commands {
            let delivered = this.update_in(cx, |this, _window, cx| {
                let Some(tab) = this
                    .state
                    .workspace
                    .tab(tab_id)
                    .filter(|tab| tab.connection_id() == Some(connection_id))
                else {
                    return false;
                };
                let Some(terminal) = tab.terminal() else {
                    return false;
                };
                terminal.update(cx, |view, cx| {
                    view.send_text(&format!("{command}\r"), cx);
                });
                true
            });
            if !delivered.unwrap_or(false) {
                break;
            }
            cx.background_executor().timer(STARTUP_INTERVAL).await;
        }
    });

    if let Some(pane) = app_state
        .workspace
        .tabs
        .iter_mut()
        .find(|tab| tab.id == tab_id && tab.connection_id() == Some(connection_id))
        .and_then(|tab| tab.session_pane_mut())
    {
        pane._startup_task = Some(task);
    }
}

#[cfg(test)]
mod tests {
    use super::build_persistence_commands;
    use transom_core::{Connection, Session, SshConnection};

    fn make_ssh_session(restore_cwd: bool, persist_tmux: bool) -> Session {
        let mut ssh = SshConnection::new("example.com", "root");
        ssh.options.restore_cwd_on_reconnect = restore_cwd;
        ssh.options.persist_tmux_session = persist_tmux;
        Session::new("test", Connection::Ssh(ssh))
    }

    #[test]
    fn restores_quoted_cwd_when_enabled() {
        let session = make_ssh_session(true, false);
        let cmds = build_persistence_commands(&session, Some("/var/log/my app's"));
        assert_eq!(cmds, vec!["cd -- '/var/log/my app'\\''s'"]);
    }

    #[test]
    fn tmux_persistence_takes_precedence_over_cwd_restore() {
        let session = make_ssh_session(true, true);
        let cmds = build_persistence_commands(&session, Some("/tmp"));
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].contains("tmux new-session -A -s transom_"));
    }

    #[test]
    fn skips_when_both_options_disabled() {
        let session = make_ssh_session(false, false);
        assert!(build_persistence_commands(&session, Some("/tmp")).is_empty());
    }
}
