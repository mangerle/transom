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
    // 只在 TTY 会话上设置：无 TTY 的 exec 调 resize 会被引擎拒绝（真机返回
    // 500），既没有意义又会在日志里留下无关告警。
    // 一次性命令可能在 resize 到达前就已退出，此时保持引擎默认尺寸即可，
    // 属于预期降级，不打扰 warn 级日志。
    if connection.tty
        && let Err(error) = api.resize_exec(&exec_id, size.cols, size.rows).await
    {
        log::debug!(
            "容器 exec 初始尺寸未生效，容器: {}，尺寸: {}x{}，原因: {error}",
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
    let read_tty = connection.tty;
    let read_task = tokio::spawn(async move {
        let mut buffer = vec![0u8; READ_CHUNK];
        let mut demux_buffer = Vec::with_capacity(READ_CHUNK);
        loop {
            match read_half.read(&mut buffer).await {
                Ok(0) => break,
                Ok(count) => {
                    let chunk = &buffer[..count];
                    if read_tty {
                        if output_tx.send_async(Ok(chunk.to_vec())).await.is_err() {
                            break;
                        }
                    } else {
                        // 非 TTY 会话：解 8 字节多路复用帧头 [stream_type, 0, 0, 0, size_be(4)]
                        demux_buffer.extend_from_slice(chunk);
                        for frame in decode_demux_frames(&mut demux_buffer) {
                            if output_tx.send_async(Ok(frame)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = output_tx.send_async(Err(error.to_string())).await;
                    break;
                }
            }
        }
        if !demux_buffer.is_empty() && output_tx.send_async(Ok(demux_buffer)).await.is_err() {
            return;
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
    let control_tty = connection.tty;
    let control_task = tokio::spawn(async move {
        while let Some(message) = control_rx.recv().await {
            match message {
                TerminalControl::Resize(size) => {
                    // 无 TTY 的 exec 不支持改尺寸：终端每次重绘都会发 Resize，
                    // 这里不拦就会按帧产生 500 告警，直接静默丢弃。
                    if !control_tty {
                        continue;
                    }
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

/// 解码 Docker HTTP Exec 非 TTY 模式下的多路复用数据帧
///
/// ### 设计初衷 (Why)
/// Docker 引擎在 `Tty: false` 模式下，标准输出与标准错误流会被多路复用打包。
/// 每个有效数据帧包含 8 字节头部：`[stream_type(1), 0, 0, 0, size_be(4)]`。
/// 若直接透传原始二进制字节流，终端或消费端会看到乱码或包含 8 字节控制头的损坏输出。
///
/// ### 核心优势
/// 纯内存原地切片提取，安全处理跨网络分片和粘包。未完整的帧头与数据会被保留在缓冲区中，
/// 等待后续数据块到达，避免数据截断。
///
/// ### 代价与局限
/// 若容器输出极高频或单次帧体积极大，缓冲区会发生适量内存搬移；调用方需合理控制读取分块大小。
pub(crate) fn decode_demux_frames(demux_buffer: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    while demux_buffer.len() >= 8 {
        let length = u32::from_be_bytes([
            demux_buffer[4],
            demux_buffer[5],
            demux_buffer[6],
            demux_buffer[7],
        ]) as usize;
        if demux_buffer.len() < 8 + length {
            break;
        }
        let frame: Vec<u8> = demux_buffer.drain(..8 + length).skip(8).collect();
        if !frame.is_empty() {
            frames.push(frame);
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_demux_frames_single_and_split() {
        let mut buffer = Vec::new();

        // 构造一个包含 "hello world" 的标准输出帧 (stream_type = 1, len = 11)
        let payload = b"hello world";
        let mut frame = vec![1u8, 0, 0, 0];
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);

        // 模拟网络分片：先传入前 6 个字节
        buffer.extend_from_slice(&frame[..6]);
        let decoded = decode_demux_frames(&mut buffer);
        assert!(decoded.is_empty());
        assert_eq!(buffer.len(), 6);

        // 传入剩余字节
        buffer.extend_from_slice(&frame[6..]);
        let decoded = decode_demux_frames(&mut buffer);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0], b"hello world");
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_decode_demux_frames_multiple_frames() {
        let mut buffer = Vec::new();

        // 构造两帧：stdout "one", stderr "two"
        let mut data = vec![1u8, 0, 0, 0, 0, 0, 0, 3];
        data.extend_from_slice(b"one");
        data.extend_from_slice(&[2u8, 0, 0, 0, 0, 0, 0, 3]);
        data.extend_from_slice(b"two");

        buffer.extend_from_slice(&data);
        let decoded = decode_demux_frames(&mut buffer);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0], b"one");
        assert_eq!(decoded[1], b"two");
        assert!(buffer.is_empty());
    }
}

