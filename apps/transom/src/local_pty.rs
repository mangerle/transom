//! 本地 PTY 的共用启动逻辑。
//!
//! 普通标签与 Windows 提权代理都必须在同一处解析 Shell、附加参数并预装集成脚本；
//! 否则两条路径会逐渐出现环境变量、rcfile 与工作目录行为不一致。

use crate::controller::local_shell::{self, LocalShellPreload};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use transom_core::protocol::shell_integration::{ShellKind, detect_shell};
use transom_core::{AppError, ErrorCode, LocalConnection, TerminalSize};

/// 已经启动并挂到伪终端上的本地 Shell。
pub(crate) struct LocalPtySession {
    pub(crate) master: Box<dyn MasterPty + Send>,
    pub(crate) child: Box<dyn Child + Send + Sync>,
    pub(crate) shell: String,
    pub(crate) exec_argv: Option<Vec<String>>,
    pub(crate) preload: LocalShellPreload,
    /// 当前方言既无法预装集成，也没有可安全注入的终端侧脚本。
    pub(crate) integration_disabled: bool,
}

/// 创建一个本地伪终端并按连接配置启动 Shell 或一次性命令。
///
/// # Errors
/// PTY 创建、Shell 解析或子进程启动失败时返回带中文上下文的 [`AppError`]。
pub(crate) fn spawn(
    local: &LocalConnection,
    size: TerminalSize,
    shell_integration: bool,
) -> Result<LocalPtySession, AppError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| {
            AppError::with_detail(ErrorCode::Io, format!("创建本地伪终端失败: {error}"))
        })?;

    let shell = resolved_shell(local);
    let exec_argv = local
        .command
        .as_ref()
        .filter(|argv| !argv.is_empty())
        .cloned();
    let mut command = match &exec_argv {
        Some(argv) => {
            let mut builder = CommandBuilder::new(&argv[0]);
            builder.args(&argv[1..]);
            builder
        }
        None => {
            let mut builder = CommandBuilder::new(&shell);
            builder.args(&local.shell_args);
            builder
        }
    };
    if let Some(dir) = &local.working_dir {
        command.cwd(dir);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for (key, value) in &local.env {
        command.env(key, value);
    }

    let preload = if shell_integration && exec_argv.is_none() {
        local_shell::prepare(&shell)
    } else {
        LocalShellPreload::none()
    };
    command.args(&preload.args);
    for (key, value) in &preload.env {
        command.env(key, value);
    }
    let integration_disabled = should_disable_integration(
        shell_integration,
        exec_argv.is_some(),
        preload.kind,
        detect_shell(&shell),
    );

    let child = pair.slave.spawn_command(command).map_err(|error| {
        AppError::with_detail(ErrorCode::Io, format!("启动本地 Shell 失败: {error}"))
    })?;
    drop(pair.slave);
    Ok(LocalPtySession {
        master: pair.master,
        child,
        shell,
        exec_argv,
        preload,
        integration_disabled,
    })
}

/// 解析配置中的 Shell；空字符串视为未配置，避免手动编辑出的脏配置变成空命令。
pub(crate) fn resolved_shell(local: &LocalConnection) -> String {
    local
        .shell
        .as_deref()
        .filter(|shell| !shell.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(transom_platform::shell::system_default_program)
}

fn should_disable_integration(
    requested: bool,
    command_mode: bool,
    preinstalled: Option<ShellKind>,
    shell_kind: ShellKind,
) -> bool {
    requested && !command_mode && preinstalled.is_none() && shell_kind == ShellKind::Other
}

#[cfg(test)]
mod tests {
    use super::{resolved_shell, should_disable_integration};
    use transom_core::LocalConnection;
    use transom_core::protocol::shell_integration::ShellKind;

    #[test]
    fn blank_shell_falls_back_to_system_default() {
        let local = LocalConnection {
            shell: Some("   ".to_string()),
            ..LocalConnection::default()
        };
        assert_eq!(
            resolved_shell(&local),
            transom_platform::shell::system_default_program()
        );
    }

    #[test]
    fn unsupported_local_shell_disables_terminal_side_probe() {
        assert!(should_disable_integration(
            true,
            false,
            None,
            ShellKind::Other
        ));
        assert!(!should_disable_integration(
            true,
            false,
            None,
            ShellKind::Fish
        ));
        assert!(!should_disable_integration(
            true,
            true,
            None,
            ShellKind::Other
        ));
    }
}
