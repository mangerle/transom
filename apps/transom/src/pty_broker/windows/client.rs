//! 提权代理的客户端：启动 runas 进程、校验管道对端并组装 TerminalIo。

use super::{BrokerConfig, BrokerReady, PipeNames, io_error};
use crate::local_pty;
use crate::pty_broker::{BROKER_ARG, read_frame, write_frame};
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;
use transom_core::{
    AppError, ErrorCode, LocalConnection, SessionCleanup, TerminalControl, TerminalControlHandle,
    TerminalIo, TerminalSize,
};
use transom_platform::named_pipe::{
    BROKER_PIPE_ACCEPT_TIMEOUT, accept_private_server, create_private_blocking_server,
    private_server_client_process_id,
};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    GetProcessId, INFINITE, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

/// 从当前标签启动提权代理并桥接成普通 [`TerminalIo`]。
pub(crate) fn open_elevated_pty(
    local: &LocalConnection,
    size: TerminalSize,
    shell_integration: bool,
) -> Result<(TerminalIo, tokio::sync::oneshot::Receiver<()>), AppError> {
    let shell = local_pty::resolved_shell(local);
    let shell_name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell.as_str())
        .to_ascii_lowercase();
    if shell_name.starts_with("wsl") {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "WSL 启动器无法通过 Windows 提权获得发行版 root；请进入发行版后使用 sudo 或 wsl -u root",
        ));
    }
    let token = uuid::Uuid::new_v4().to_string();
    let names = PipeNames::new(&token);
    let config_server = create_server(&names.config, "配置")?;
    let input_server = create_server(&names.input, "输入")?;
    let output_server = create_server(&names.output, "输出")?;
    let control_server = create_server(&names.control, "控制")?;

    let mut process = BrokerProcessGuard::launch(&token)?;
    accept_private_server(&config_server, BROKER_PIPE_ACCEPT_TIMEOUT)
        .map_err(|error| io_error("等待提权 PTY 代理连接配置管道", error))?;
    verify_pipe_peer(&config_server, process.process_id()?)?;
    let mut config_channel = config_server;
    write_frame(
        &mut config_channel,
        &BrokerConfig {
            shell: local.shell.clone(),
            shell_args: local.shell_args.clone(),
            working_dir: local.working_dir.clone(),
            command: local.command.clone(),
            env: local.env.clone(),
            size,
            shell_integration,
        },
    )
    .map_err(|error| io_error("发送提权 PTY 代理配置", error))?;
    let ready: BrokerReady = read_frame(&mut config_channel)
        .map_err(|error| io_error("读取提权 PTY 代理握手", error))?;
    drop(config_channel);

    accept_private_server(&input_server, BROKER_PIPE_ACCEPT_TIMEOUT)
        .map_err(|error| io_error("等待提权 PTY 输入管道", error))?;
    verify_pipe_peer(&input_server, process.process_id()?)?;
    accept_private_server(&output_server, BROKER_PIPE_ACCEPT_TIMEOUT)
        .map_err(|error| io_error("等待提权 PTY 输出管道", error))?;
    verify_pipe_peer(&output_server, process.process_id()?)?;
    accept_private_server(&control_server, BROKER_PIPE_ACCEPT_TIMEOUT)
        .map_err(|error| io_error("等待提权 PTY 控制管道", error))?;
    verify_pipe_peer(&control_server, process.process_id()?)?;

    let (control, control_rx) = TerminalControlHandle::channel(16);
    spawn_control_writer(control_server, control_rx)?;
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    spawn_process_waiter(process.handle(), exit_tx)?;

    let process_handle = process.handle();
    let mut io = TerminalIo::new(Box::new(output_server), Box::new(input_server), control);
    if let Some(kind) = ready.integration {
        io = io.with_preinstalled_integration(kind);
    } else if ready.integration_disabled {
        io = io.without_shell_integration();
    }
    io.cleanup = SessionCleanup::new(move || terminate_broker_process(&process_handle));
    process.disarm();
    Ok((io, exit_rx))
}

fn spawn_control_writer(
    mut control: File,
    mut control_rx: tokio::sync::mpsc::Receiver<TerminalControl>,
) -> Result<(), AppError> {
    std::thread::Builder::new()
        .name("transom-elevated-pty-control-parent".to_string())
        .spawn(move || {
            while let Some(message) = control_rx.blocking_recv() {
                let frame = match message {
                    TerminalControl::Resize(size) => {
                        let mut frame = Vec::with_capacity(5);
                        frame.push(1);
                        frame.extend_from_slice(&size.cols.to_le_bytes());
                        frame.extend_from_slice(&size.rows.to_le_bytes());
                        frame
                    }
                    TerminalControl::Close => vec![2],
                };
                let close = frame.first() == Some(&2);
                if control.write_all(&frame).is_err() || control.flush().is_err() {
                    break;
                }
                if close {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|error| io_error("创建提权 PTY 控制线程", error))
}

fn spawn_process_waiter(
    process: Arc<OwnedHandle>,
    exit_tx: tokio::sync::oneshot::Sender<()>,
) -> Result<(), AppError> {
    std::thread::Builder::new()
        .name("transom-elevated-pty-wait".to_string())
        .spawn(move || {
            // 安全性：句柄由 Arc 保证在等待期间有效。
            unsafe {
                WaitForSingleObject(raw_handle(&process), INFINITE);
            }
            let _ = exit_tx.send(());
        })
        .map(|_| ())
        .map_err(|error| io_error("创建提权 PTY 等待线程", error))
}

struct BrokerProcessGuard {
    handle: Arc<OwnedHandle>,
    armed: bool,
}

impl BrokerProcessGuard {
    fn launch(token: &str) -> Result<Self, AppError> {
        let executable = std::env::current_exe()
            .map_err(|error| io_error("获取 Transom 可执行文件路径", error))?;
        let executable = HSTRING::from(executable.as_os_str());
        let parameters = HSTRING::from(format!("{BROKER_ARG} {token}"));
        let operation = HSTRING::from("runas");
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(operation.as_ptr()),
            lpFile: PCWSTR(executable.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // 安全性：结构体字段与生命周期均在调用返回前有效。
        unsafe { ShellExecuteExW(&mut info) }.map_err(|error| {
            let code = win32_error_code(&error);
            AppError::with_detail(
                if code == 1223 {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::Io
                },
                format!("启动管理员 PTY 代理失败，Windows 错误码: {code}，原因: {error}"),
            )
        })?;
        if info.hProcess.is_invalid() {
            return Err(AppError::with_detail(
                ErrorCode::Io,
                "管理员 PTY 代理未返回进程句柄",
            ));
        }
        // 安全性：ShellExecuteExW 返回的句柄所有权转移给 OwnedHandle。
        let handle = unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) };
        Ok(Self {
            handle: Arc::new(handle),
            armed: true,
        })
    }

    fn handle(&self) -> Arc<OwnedHandle> {
        self.handle.clone()
    }

    fn process_id(&self) -> Result<u32, AppError> {
        // 安全性：句柄由 OwnedHandle 保证有效。
        let process_id = unsafe { GetProcessId(raw_handle(&self.handle)) };
        if process_id == 0 {
            return Err(io_error(
                "读取提权 PTY 代理进程 id",
                io::Error::last_os_error(),
            ));
        }
        Ok(process_id)
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for BrokerProcessGuard {
    fn drop(&mut self) {
        if self.armed {
            terminate_broker_process(&self.handle);
        }
    }
}

fn terminate_broker_process(process: &Arc<OwnedHandle>) {
    let handle = raw_handle(process);
    // 安全性：只操作本进程拥有的代理进程句柄。
    unsafe {
        if WaitForSingleObject(handle, 500) != WAIT_OBJECT_0 {
            let _ = TerminateProcess(handle, 1);
            WaitForSingleObject(handle, INFINITE);
        }
    }
}

fn raw_handle(handle: &Arc<OwnedHandle>) -> HANDLE {
    HANDLE(handle.as_raw_handle() as *mut _)
}

fn win32_error_code(error: &windows::core::Error) -> u32 {
    let code = error.code().0 as u32;
    if code & 0xffff_0000 == 0x8007_0000 {
        code & 0xffff
    } else {
        code
    }
}

fn create_server(name: &str, purpose: &str) -> Result<File, AppError> {
    create_private_blocking_server(name)
        .map_err(|error| io_error(&format!("创建提权 PTY {purpose}管道"), error))
}

fn verify_pipe_peer(server: &File, expected_process_id: u32) -> Result<(), AppError> {
    let actual = private_server_client_process_id(server)
        .map_err(|error| io_error("读取提权 PTY 管道客户端进程", error))?;
    if actual != expected_process_id {
        return Err(AppError::with_detail(
            ErrorCode::PermissionDenied,
            format!(
                "提权 PTY 管道客户端身份不匹配，期望进程: {expected_process_id}，实际: {actual}"
            ),
        ));
    }
    Ok(())
}
