//! 进程采样与信号控制操作。

use super::stats::COMMAND_TIMEOUT;
use tokio::process::Command;
use transom_core::{AppError, AppEvent, ConnectionId, ErrorCode, TabId};
use transom_net::SshRemote;

/// 进程采样/信号控制的目标：SSH 会话走远端 exec 通道，本地终端跑本机命令。
///
/// 串口与 Telnet 不参与进程面板：控制器只在远端已登记或调用方确认本地终端时
/// 才会收到命令，按 `remote` 是否为 `Some` 选择执行位置。
pub(crate) struct ProcessTarget {
    pub(crate) remote: Option<SshRemote>,
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) event_tx: flume::Sender<AppEvent>,
}

/// 按 CPU 降序取前 50 个进程的一行采样命令。
///
/// 与 `ps` 默认输出不同，`--sort=-%cpu` 让最活跃的进程一定在面板顶部；
/// `head -n 51` 保留表头在内的 51 行，远端输出量可控。
const PROCESS_SAMPLE_COMMAND: &str =
    "ps -eo pid,ppid,user,%cpu,%mem,rss,comm --sort=-%cpu | head -n 51";

pub(crate) async fn run_process_sample(target: ProcessTarget) {
    let result = match &target.remote {
        Some(remote) => {
            match tokio::time::timeout(
                COMMAND_TIMEOUT,
                remote.exec_command_capture(PROCESS_SAMPLE_COMMAND),
            )
            .await
            {
                Ok(Ok(output)) => Ok(output.stdout),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(command_timeout("进程采样")),
            }
        }
        None => local_command_output(PROCESS_SAMPLE_COMMAND).await,
    };
    let event = match result {
        Ok(output) => {
            let processes = transom_core::system_stats::parse_processes(&output);
            AppEvent::ProcessesSampled {
                tab_id: target.tab_id,
                connection_id: target.connection_id,
                processes,
            }
        }
        Err(error) => AppEvent::ProcessesSampleFailed {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            error,
        },
    };
    let _ = target.event_tx.send_async(event).await;
}

pub(crate) async fn run_process_kill(target: ProcessTarget, pid: u32, force: bool) {
    // PID 1 是 init/systemd：误杀会让整台远端主机失去服务管理能力，
    // 面板上即便手工伪造请求也必须在执行层拦下。
    let error = if pid <= 1 {
        Some(AppError::with_detail(
            ErrorCode::Configuration,
            "拒绝向 PID 1（init/systemd）发送信号".to_string(),
        ))
    } else {
        let signal = if force { "-9" } else { "-15" };
        match &target.remote {
            Some(remote) => {
                let command = format!("kill {signal} {pid}");
                match tokio::time::timeout(COMMAND_TIMEOUT, remote.exec_command_capture(&command))
                    .await
                {
                    Ok(Ok(output)) if output.exit_status == Some(0) => None,
                    Ok(Ok(output)) => Some(AppError::with_detail(
                        ErrorCode::ForwardFailed,
                        format!("远端拒绝终止进程 {pid}，错误输出: {}", output.stderr.trim()),
                    )),
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some(command_timeout("终止进程")),
                }
            }
            None => match local_kill(pid, signal).await {
                Ok(()) => None,
                Err(reason) => Some(AppError::with_detail(ErrorCode::Io, reason)),
            },
        }
    };
    let _ = target
        .event_tx
        .send_async(AppEvent::ProcessKilled {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            pid,
            error,
        })
        .await;
}

fn command_timeout(action: &str) -> AppError {
    AppError::with_detail(
        ErrorCode::Timeout,
        format!("{action}超时，已等待 {} 秒", COMMAND_TIMEOUT.as_secs()),
    )
}

async fn local_command_output(command: &str) -> Result<String, AppError> {
    let mut child = Command::new("sh");
    child.arg("-c").arg(command).kill_on_drop(true);
    match tokio::time::timeout(COMMAND_TIMEOUT, child.output()).await {
        Ok(Ok(output)) => Ok(String::from_utf8_lossy(&output.stdout).to_string()),
        Ok(Err(error)) => Err(AppError::with_detail(
            ErrorCode::Io,
            format!("执行本地进程采样失败: {error}"),
        )),
        Err(_) => Err(command_timeout("本地进程采样")),
    }
}

async fn local_kill(pid: u32, signal: &str) -> Result<(), String> {
    let mut child = Command::new("kill");
    child.arg(signal).arg(pid.to_string()).kill_on_drop(true);
    let status = tokio::time::timeout(COMMAND_TIMEOUT, child.status())
        .await
        .map_err(|_| "本地 kill 命令超时".to_string())?
        .map_err(|error| format!("执行本地 kill 失败: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("本地 kill 返回失败状态: {status}"))
    }
}
