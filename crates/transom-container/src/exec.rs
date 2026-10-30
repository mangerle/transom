//! 交互式 exec：把容器内的 TTY 会话接成与 SSH/PTY 同形的 [`TerminalIo`]。
//!
//! 数据通路（Why）：
//! - HTTP 后端：`POST /exec` 创建带 TTY 的 exec，`/start` 通过 HTTP Upgrade
//!   取得原始双向流，再拆成读/写两个异步任务，经有界通道交给阻塞式终端
//!   读写线程；
//! - CLI 后端：WSL/WSLC 无法直连 socket，直接拉起 `docker exec -it`，
//!   用管道与子进程交换字节；resize 不支持，显式记录能力差异；
//! - 两条路径都把任务/线程句柄纳入 [`SessionCleanup`]，会话视图回收时
//!   终止子进程与任务，不留孤儿。

use crate::cli::CliBackend;
use crate::engine::Engine;
use crate::error::ContainerError;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, watch};
use transom_core::{
    ContainerConnection, SessionCleanup, TerminalControl, TerminalControlHandle, TerminalIo,
    TerminalSize,
};

use crate::bridge::{BRIDGE_CAPACITY, input_channel, output_channel};

/// 单次读取块大小：与终端读线程保持一致。
const READ_CHUNK: usize = 16 * 1024;

/// 打开容器内交互式终端。
///
/// # Errors
/// 引擎不可用、容器不存在、exec 创建或升级失败、CLI 子进程无法启动时返回
/// [`ContainerError`]；具体码由控制器映射为 `AppError`。
pub async fn open_exec(
    engine: &Engine,
    connection: &ContainerConnection,
    size: TerminalSize,
) -> Result<(TerminalIo, oneshot::Receiver<()>), ContainerError> {
    if let Some(cli) = engine.cli_backend_ref() {
        return cli_exec(cli, connection);
    }
    http_exec(engine, connection, size).await
}

/// 保证会话结束时所有异步任务都被中止。
struct TaskGuard {
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

async fn http_exec(
    engine: &Engine,
    connection: &ContainerConnection,
    size: TerminalSize,
) -> Result<(TerminalIo, oneshot::Receiver<()>), ContainerError> {
    let api = engine.http()?.clone();
    let exec_id = api
        .create_exec(
            &connection.container,
            &connection.command_argv(),
            connection.user.as_deref(),
            connection.working_dir.as_deref(),
            &connection.env,
            connection.tty,
        )
        .await?;
    let stream = api.start_exec_stream(&exec_id).await?;
    // 初始尺寸：exec 创建时使用引擎默认尺寸，启动后立即校正一次。
    if let Err(error) = api.resize_exec(&exec_id, size.cols, size.rows).await {
        log::warn!(
            "容器 exec 初始尺寸设置失败，容器: {}，尺寸: {}x{}，原因: {error}",
            connection.container,
            size.cols,
            size.rows
        );
    }

    let (mut read_half, mut write_half) = tokio::io::split(stream);
    let (output_tx, output_reader) = output_channel(BRIDGE_CAPACITY);
    let (input_writer, input_rx) = input_channel(BRIDGE_CAPACITY);
    let (control, mut control_rx) = TerminalControlHandle::channel(16);
    let (exit_tx, exit_rx) = oneshot::channel();
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let read_api = api.clone();
    let read_exec_id = exec_id.clone();
    let read_container = connection.container.clone();
    let read_task = tokio::spawn(async move {
        let mut buffer = vec![0u8; READ_CHUNK];
        loop {
            match read_half.read(&mut buffer).await {
                Ok(0) => break,
                Ok(count) => {
                    if output_tx
                        .send_async(Ok(buffer[..count].to_vec()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = output_tx.send_async(Err(error.to_string())).await;
                    break;
                }
            }
        }
        // EOF 后查询退出码：只用于日志，界面只关心会话已结束。
        match read_api.inspect_exec(&read_exec_id).await {
            Ok(inspect) => log::info!(
                "容器 exec 已结束，容器: {read_container}，退出码: {:?}",
                inspect.exit_code
            ),
            Err(error) => {
                log::debug!("容器 exec 结束但退出码查询失败，容器: {read_container}，原因: {error}")
            }
        }
        let _ = exit_tx.send(());
    });

    let write_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                item = input_rx.recv_async() => {
                    match item {
                        Ok(bytes) => {
                            if write_half.write_all(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
        let _ = write_half.shutdown().await;
    });

    let control_api = api.clone();
    let control_exec_id = exec_id.clone();
    let control_task = tokio::spawn(async move {
        while let Some(message) = control_rx.recv().await {
            match message {
                TerminalControl::Resize(size) => {
                    if let Err(error) = control_api
                        .resize_exec(&control_exec_id, size.cols, size.rows)
                        .await
                    {
                        log::debug!(
                            "容器 exec 尺寸调整失败，exec: {control_exec_id}，原因: {error}"
                        );
                    }
                }
                TerminalControl::Close => {
                    let _ = shutdown_tx.send(true);
                    break;
                }
            }
        }
    });

    let guard = TaskGuard {
        handles: vec![read_task, write_task, control_task],
    };
    let mut io = TerminalIo::new(Box::new(output_reader), Box::new(input_writer), control);
    // 清理守卫随会话视图释放：中止读/写/控制任务，连接随之关闭。
    io.cleanup = SessionCleanup::new(move || drop(guard));
    Ok((io, exit_rx))
}

/// CLI 兜底的子进程会话。
struct CliSession {
    child: Arc<Mutex<std::process::Child>>,
    wait_thread: Option<std::thread::JoinHandle<()>>,
    control_thread: Option<std::thread::JoinHandle<()>>,
    pump_threads: Vec<std::thread::JoinHandle<()>>,
}

impl Drop for CliSession {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.wait_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.control_thread.take() {
            let _ = handle.join();
        }
        for handle in self.pump_threads.drain(..) {
            let _ = handle.join();
        }
    }
}

fn cli_exec(
    cli: &CliBackend,
    connection: &ContainerConnection,
) -> Result<(TerminalIo, oneshot::Receiver<()>), ContainerError> {
    let mut args = vec!["exec".to_string(), "-i".to_string(), "-t".to_string()];
    if let Some(user) = connection.user.as_deref() {
        args.push("--user".to_string());
        args.push(user.to_string());
    }
    if let Some(dir) = connection.working_dir.as_deref() {
        args.push("--workdir".to_string());
        args.push(dir.to_string());
    }
    for env in &connection.env {
        args.push("--env".to_string());
        args.push(env.clone());
    }
    args.push(connection.container.clone());
    args.extend(connection.command_argv());

    // CLI 兜底直接用标准库子进程：管道实现阻塞式 Read/Write，正好符合
    // TerminalIo 的读写线程模型，无需再包一层异步桥。
    let mut command = std::process::Command::new(&cli.spec.program);
    command
        .args(&cli.spec.prefix)
        .args(&args)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|error| ContainerError::Cli {
        command: cli.spec.describe(&args),
        message: error.to_string(),
    })?;
    let stdin = child.stdin.take().ok_or_else(|| ContainerError::Cli {
        command: cli.spec.describe(&args),
        message: "子进程标准输入不可用".to_string(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| ContainerError::Cli {
        command: cli.spec.describe(&args),
        message: "子进程标准输出不可用".to_string(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| ContainerError::Cli {
        command: cli.spec.describe(&args),
        message: "子进程标准错误不可用".to_string(),
    })?;

    let child = Arc::new(Mutex::new(child));
    let (output_tx, output_reader) = output_channel(BRIDGE_CAPACITY);
    // stderr 与 stdout 合流：终端只有一个读取端，CLI 的错误文本也要可见。
    let pump_threads = vec![
        spawn_pipe_pump(stdout, output_tx.clone()),
        spawn_pipe_pump(stderr, output_tx),
    ];
    let (control, mut control_rx) = TerminalControlHandle::channel(16);
    let (exit_tx, exit_rx) = oneshot::channel();

    let wait_child = Arc::clone(&child);
    let wait_thread = std::thread::Builder::new()
        .name("transom-container-cli-wait".to_string())
        .spawn(move || {
            loop {
                let status = wait_child
                    .lock()
                    .map_err(|error| error.to_string())
                    .and_then(|mut child| child.try_wait().map_err(|error| error.to_string()));
                match status {
                    Ok(Some(status)) => {
                        log::info!("容器 CLI 会话已结束，状态: {status}");
                        break;
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                    Err(error) => {
                        log::warn!("等待容器 CLI 会话退出失败，原因: {error}");
                        break;
                    }
                }
            }
            let _ = exit_tx.send(());
        })
        .map_err(|error| ContainerError::Cli {
            command: cli.spec.describe(&args),
            message: format!("无法创建等待线程: {error}"),
        })?;

    let control_child = Arc::clone(&child);
    let control_thread = std::thread::Builder::new()
        .name("transom-container-cli-control".to_string())
        .spawn(move || {
            while let Some(message) = control_rx.blocking_recv() {
                match message {
                    TerminalControl::Resize(_) => {
                        // CLI 后端没有 resize 接口；能力矩阵已标记，不重复刷日志。
                    }
                    TerminalControl::Close => {
                        if let Ok(mut child) = control_child.lock() {
                            let _ = child.kill();
                        }
                        break;
                    }
                }
            }
        })
        .map_err(|error| ContainerError::Cli {
            command: cli.spec.describe(&args),
            message: format!("无法创建控制线程: {error}"),
        })?;

    let session = CliSession {
        child,
        wait_thread: Some(wait_thread),
        control_thread: Some(control_thread),
        pump_threads,
    };
    let mut io = TerminalIo::new(Box::new(output_reader), Box::new(stdin), control);
    io.cleanup = SessionCleanup::new(move || drop(session));
    Ok((io, exit_rx))
}

/// 把子进程管道的数据泵入桥接通道；读取结束或对端关闭时线程自行退出。
fn spawn_pipe_pump<R: Read + Send + 'static>(
    mut reader: R,
    sender: flume::Sender<crate::bridge::BridgeItem>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("transom-container-cli-pipe".to_string())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(Ok(buffer[..count].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error.to_string()));
                        break;
                    }
                }
            }
        })
        .unwrap_or_else(|error| {
            log::warn!("创建容器 CLI 管道线程失败，原因: {error}");
            // 返回一个立即结束的线程句柄：调用方仍可统一回收。
            std::thread::spawn(|| {})
        })
}
