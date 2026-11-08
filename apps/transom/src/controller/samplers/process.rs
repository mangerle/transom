//! 进程采样与信号控制操作。

use super::stats::COMMAND_TIMEOUT;
use std::sync::Arc;
use transom_core::{AppError, AppEvent, ConnectionId, ErrorCode, HostAppEvent, ProcessInfo, TabId};
use transom_net::SshRemote;
use transom_platform::system::{ProcessInfo as HostProcessInfo, ProcessSampler};

/// 进程采样/信号控制的目标：SSH 会话走远端 exec 通道，本地终端使用宿主采样器。
///
/// 串口与 Telnet 不参与进程面板：控制器只在远端已登记或调用方确认本地终端时
/// 才会收到命令，按 `remote` 是否为 `Some` 选择执行位置。
pub(crate) struct ProcessTarget {
    pub(crate) remote: Option<SshRemote>,
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) event_tx: flume::Sender<AppEvent>,
    pub(crate) local: Arc<parking_lot::Mutex<ProcessSampler>>,
}

pub(crate) async fn run_process_sample(target: ProcessTarget) {
    // 不排序、不截断：进程表要完整可搜，排序在界面侧按当前列完成，
    // 远端多排一次全表既浪费又和界面排序口径重复。
    let result = match &target.remote {
        Some(remote) => remote_sample_command(
            remote,
            "ps -eo pid,ppid,user,%cpu,%mem,rss,comm,args",
            "进程采样",
        )
        .await
        .map(|output| transom_net::system_stats::parse_processes(&output)),
        None => sample_local_processes(&target.local).await,
    };

    let event: AppEvent = match result {
        Ok(processes) => HostAppEvent::ProcessesSampled {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            processes,
        }
        .into(),
        Err(error) => HostAppEvent::ProcessesSampleFailed {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            error,
        }
        .into(),
    };
    let _ = target.event_tx.send_async(event).await;
}

/// 折叠态采样：只取进程总数，用窄列输出把每轮传输量压到数 KB。
pub(crate) async fn run_process_count(target: ProcessTarget) {
    let result = match &target.remote {
        Some(remote) => remote_sample_command(remote, "ps -eo pid=", "进程计数采样")
            .await
            .map(|output| transom_net::system_stats::parse_process_count(&output)),
        None => sample_local_processes(&target.local)
            .await
            .map(|processes| u32::try_from(processes.len()).unwrap_or(u32::MAX)),
    };

    let event: AppEvent = match result {
        Ok(count) => HostAppEvent::ProcessCountSampled {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            count,
        }
        .into(),
        Err(error) => HostAppEvent::ProcessesSampleFailed {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            error,
        }
        .into(),
    };
    let _ = target.event_tx.send_async(event).await;
}

/// 执行远端采样命令并取回标准输出。
async fn remote_sample_command(
    remote: &SshRemote,
    command: &str,
    action: &str,
) -> Result<String, AppError> {
    match tokio::time::timeout(COMMAND_TIMEOUT, remote.exec_command_capture(command)).await {
        Ok(Ok(output)) => Ok(output.stdout),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(command_timeout(action)),
    }
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
            None => {
                let sampler = Arc::clone(&target.local);
                match tokio::task::spawn_blocking(move || sampler.lock().kill(pid, force)).await {
                    Ok(Ok(())) => None,
                    Ok(Err(reason)) => Some(AppError::with_detail(ErrorCode::Io, reason)),
                    Err(error) => Some(AppError::with_detail(
                        ErrorCode::Io,
                        format!("本地进程终止任务异常终止: {error}"),
                    )),
                }
            }
        }
    };
    let _ = target
        .event_tx
        .send_async(
            HostAppEvent::ProcessKilled {
                tab_id: target.tab_id,
                connection_id: target.connection_id,
                pid,
                error,
            }
            .into(),
        )
        .await;
}

fn command_timeout(action: &str) -> AppError {
    AppError::with_detail(
        ErrorCode::Timeout,
        format!("{action}超时，已等待 {} 秒", COMMAND_TIMEOUT.as_secs()),
    )
}

async fn sample_local_processes(
    sampler: &Arc<parking_lot::Mutex<ProcessSampler>>,
) -> Result<Vec<ProcessInfo>, AppError> {
    let sampler = Arc::clone(sampler);
    tokio::task::spawn_blocking(move || sampler.lock().sample())
        .await
        .map(|processes| processes.into_iter().map(map_process).collect())
        .map_err(|error| {
            AppError::with_detail(ErrorCode::Io, format!("本地进程采样任务异常终止: {error}"))
        })
}

fn map_process(process: HostProcessInfo) -> ProcessInfo {
    ProcessInfo {
        pid: process.pid,
        ppid: process.ppid,
        user: process.user,
        cpu_percent: process.cpu_percent,
        mem_percent: process.mem_percent,
        rss_bytes: process.rss_bytes,
        cmdline: process.cmdline,
        args: process.args,
    }
}
