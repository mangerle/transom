//! 容器日志流：非 TTY 容器需要解 8 字节多路复用帧，TTY 容器是裸流。
//!
//! 协议（Why）：Docker/Podman 对非 TTY 容器把 stdout/stderr 拆成
//! `[stream:u8, 0, 0, 0, len:u32-be, payload]` 帧；TTY 容器只有一条流，
//! 服务端不再分帧。判错会把二进制头当正文显示，因此先检视容器 `Config.Tty`。

use crate::bridge::BridgeItem;
use crate::engine::Engine;
use crate::error::ContainerError;
use http_body_util::BodyExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;

use crate::http;

/// 多路复用帧头长度。
const MULTIPLEX_HEADER: usize = 8;
/// 单帧上限：异常长度直接判协议错误，避免一次分配巨大缓冲。
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// 取消检查节拍：无日志到达时也要能及时停下跟随任务。
const CANCEL_TICK: Duration = Duration::from_millis(250);

/// 日志输出回调：收到的是已经合并、可直接写入终端的字节。
pub type LogSink<'a> = dyn FnMut(Vec<u8>) + Send + 'a;

/// 拉取容器日志；`follow` 为真时持续到取消或容器结束，`timestamps` 为真时带上服务端时间戳。
///
/// `tail` 是要取的尾部行数，`0` 表示全量：引擎把数值 0 理解为「最后 0 行」，
/// 真机验证过这会返回空响应，因此这里统一翻译成引擎的 `all` 语义。
///
/// # Errors
/// 引擎不支持、容器不存在、连接中断或协议帧异常时返回 [`ContainerError`]。
pub async fn stream_logs(
    engine: &Engine,
    container: &str,
    tail: usize,
    follow: bool,
    timestamps: bool,
    cancel: &Arc<AtomicBool>,
    sink: &mut LogSink<'_>,
) -> Result<(), ContainerError> {
    if let Some(cli) = engine.cli_backend_ref() {
        return cli_logs(cli, container, tail, follow, timestamps, cancel, sink).await;
    }
    let tail = tail_token(tail);
    let api = engine.http()?;
    let tty = api.container_tty(container).await?;
    let path = format!(
        "/containers/{}/logs?stdout=1&stderr=1&tail={tail}&timestamps={timestamps}&follow={}",
        http::percent_encode(container),
        u8::from(follow)
    );
    let response = http::request(
        api.transport_ref(),
        hyper::Method::GET,
        &api.versioned_path(&path),
        bytes::Bytes::new(),
        None,
    )
    .await?;
    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        return Err(http::status_error(status, "读取容器日志", body, container));
    }

    let mut body = std::pin::pin!(response.body);
    let mut demux = Demux::new(tty);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let frame = tokio::select! {
            frame = body.frame() => frame,
            _ = tokio::time::sleep(CANCEL_TICK) => continue,
        };
        let Some(frame) = frame else {
            break;
        };
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "读取容器日志".to_string(),
            message: error.to_string(),
        })?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let mut output = Vec::with_capacity(data.len());
        demux.feed(&data, &mut output)?;
        if !output.is_empty() {
            sink(output);
        }
    }
    Ok(())
}

/// 多容器联合日志流的 ANSI 终端高对比度色彩调色板。
const ANSI_COLOR_PALETTE: [&str; 6] = [
    "\x1b[36m", // 青色
    "\x1b[32m", // 绿色
    "\x1b[33m", // 黄色
    "\x1b[35m", // 洋红色
    "\x1b[34m", // 蓝色
    "\x1b[96m", // 亮青色
];
const ANSI_RESET: &str = "\x1b[0m";

/// 针对单条流为每行开头注入服务/容器前缀。
#[derive(Debug)]
pub struct LinePrefixer {
    prefix: Vec<u8>,
    at_line_start: bool,
}

impl LinePrefixer {
    /// 构造带容器/服务名前缀的行格式化器（默认无色）。
    #[allow(dead_code)]
    pub fn new(name: &str) -> Self {
        Self {
            prefix: format!("[{name}] ").into_bytes(),
            at_line_start: true,
        }
    }

    /// 构造带专属 ANSI 终端高亮色彩的容器/服务名前缀格式化器。
    pub fn with_color(name: &str, color_index: usize) -> Self {
        let color = ANSI_COLOR_PALETTE[color_index % ANSI_COLOR_PALETTE.len()];
        Self {
            prefix: format!("{color}[{name}]{ANSI_RESET} ").into_bytes(),
            at_line_start: true,
        }
    }

    /// 处理输入的原始字节块，返回每行注入前缀后的字节块。
    pub fn process(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len() + 64);
        for &byte in input {
            if self.at_line_start {
                out.extend_from_slice(&self.prefix);
                self.at_line_start = false;
            }
            out.push(byte);
            if byte == b'\n' {
                self.at_line_start = true;
            }
        }
        out
    }
}

/// 聚合拉取多个容器的联合日志；多路流合并推送到单一输出端，每行带彩色容器名前缀。
///
/// # Errors
/// 引擎错误或协议异常时返回 [`ContainerError`]。
pub async fn stream_project_logs(
    engine: &Engine,
    containers: Vec<String>,
    tail: usize,
    follow: bool,
    timestamps: bool,
    cancel: &Arc<AtomicBool>,
    sink: &mut LogSink<'_>,
) -> Result<(), ContainerError> {
    if containers.is_empty() {
        return Ok(());
    }

    let (tx, rx) = flume::bounded::<Vec<u8>>(64);
    let mut handles = Vec::with_capacity(containers.len());

    for (color_idx, name) in containers.into_iter().enumerate() {
        let engine = engine.clone();
        let cancel = Arc::clone(cancel);
        let tx = tx.clone();
        let handle = tokio::spawn(async move {
            let mut prefixer = LinePrefixer::with_color(&name, color_idx);
            let mut sub_sink = |chunk: Vec<u8>| {
                let formatted = prefixer.process(&chunk);
                let _ = tx.send(formatted);
            };
            let _ = stream_logs(
                &engine,
                &name,
                tail,
                follow,
                timestamps,
                &cancel,
                &mut sub_sink,
            )
            .await;
        });
        handles.push(handle);
    }
    drop(tx);

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let item = tokio::select! {
            recv = rx.recv_async() => recv,
            _ = tokio::time::sleep(CANCEL_TICK) => continue,
        };
        match item {
            Ok(chunk) => {
                if !chunk.is_empty() {
                    sink(chunk);
                }
            }
            Err(_) => break,
        }
    }

    for handle in handles {
        handle.abort();
    }
    Ok(())
}

/// 多路复用解帧器。
struct Demux {
    raw: bool,
    buffer: Vec<u8>,
    stderr_prefix: bool,
}

impl Demux {
    fn new(raw: bool) -> Self {
        Self {
            raw,
            buffer: Vec::with_capacity(8192),
            stderr_prefix: true,
        }
    }

    /// 消费一段流数据，把可输出的正文追加到 `output`。
    fn feed(&mut self, data: &[u8], output: &mut Vec<u8>) -> Result<(), ContainerError> {
        if self.raw {
            output.extend_from_slice(data);
            return Ok(());
        }
        self.buffer.extend_from_slice(data);
        loop {
            if self.buffer.len() < MULTIPLEX_HEADER {
                return Ok(());
            }
            let stream = self.buffer[0];
            let length = u32::from_be_bytes([
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
                self.buffer[7],
            ]) as usize;
            if length > MAX_FRAME_BYTES {
                return Err(ContainerError::Decode {
                    operation: "解析容器日志帧".to_string(),
                    message: format!("帧长度异常: {length} 字节"),
                });
            }
            if self.buffer.len() < MULTIPLEX_HEADER + length {
                return Ok(());
            }
            let payload: Vec<u8> = self
                .buffer
                .drain(..MULTIPLEX_HEADER + length)
                .skip(MULTIPLEX_HEADER)
                .collect();
            if payload.is_empty() {
                continue;
            }
            if stream == 2 && self.stderr_prefix {
                output.extend_from_slice(b"[stderr] ");
            }
            output.extend_from_slice(&payload);
        }
    }
}

/// 把界面的尾部行数翻译成引擎的 `tail` 取值。
///
/// 设计原理（Why）：0 在 REST 与 CLI 里都表示「最后 0 行」而不是「全部」，
/// 直接透传会让「看全部日志」的入口拿到空结果；`all` 是两种后端共同的语义。
fn tail_token(tail: usize) -> String {
    if tail == 0 {
        "all".to_string()
    } else {
        tail.to_string()
    }
}

async fn cli_logs(
    cli: &crate::cli::CliBackend,
    container: &str,
    tail: usize,
    follow: bool,
    timestamps: bool,
    cancel: &Arc<AtomicBool>,
    sink: &mut LogSink<'_>,
) -> Result<(), ContainerError> {
    let mut args = vec!["logs".to_string(), "--tail".to_string(), tail_token(tail)];
    if timestamps {
        args.push("--timestamps".to_string());
    }
    if follow {
        args.push("--follow".to_string());
    }
    args.push(container.to_string());
    let mut command = cli.command(&args);
    command
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|error| ContainerError::Cli {
        command: cli.spec.describe(&args),
        message: error.to_string(),
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (sender, receiver) = flume::bounded::<BridgeItem>(16);
    let mut pumps = Vec::with_capacity(2);
    if let Some(stdout) = stdout {
        pumps.push(tokio::spawn(pump_async(stdout, sender.clone())));
    }
    if let Some(stderr) = stderr {
        pumps.push(tokio::spawn(pump_async(stderr, sender.clone())));
    }
    drop(sender);

    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            break Ok(());
        }
        match tokio::time::timeout(CANCEL_TICK, receiver.recv_async()).await {
            Ok(Ok(Ok(bytes))) => sink(bytes),
            Ok(Ok(Err(message))) => {
                break Err(ContainerError::Http {
                    operation: "读取 CLI 日志".to_string(),
                    message,
                });
            }
            Ok(Err(_)) => break Ok(()),
            Err(_) => {}
        }
    };
    for pump in pumps {
        pump.abort();
    }
    let _ = child.wait().await;
    result
}

async fn pump_async<R>(mut reader: R, sender: flume::Sender<BridgeItem>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buffer = vec![0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if sender
                    .send_async(Ok(buffer[..count].to_vec()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send_async(Err(error.to_string())).await;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Demux, tail_token};

    fn frame(stream: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + payload.len());
        bytes.push(stream);
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn multiplexed_frames_are_split_and_stderr_marked() {
        let mut demux = Demux::new(false);
        let mut input = frame(1, b"hello ");
        input.extend(frame(2, b"oops\n"));
        input.extend(frame(1, b"world\n"));
        let mut output = Vec::new();
        demux.feed(&input, &mut output).expect("解帧必须成功");
        assert_eq!(output, b"hello [stderr] oops\nworld\n");
    }

    #[test]
    fn partial_frames_wait_for_rest_of_payload() {
        let mut demux = Demux::new(false);
        let full = frame(1, b"abcdef");
        let mut output = Vec::new();
        demux.feed(&full[..5], &mut output).expect("半帧不报错");
        assert!(output.is_empty());
        demux.feed(&full[5..], &mut output).expect("补齐后解帧");
        assert_eq!(output, b"abcdef");
    }

    #[test]
    fn raw_stream_is_passed_through() {
        let mut demux = Demux::new(true);
        let mut output = Vec::new();
        demux
            .feed(b"plain log\n", &mut output)
            .expect("裸流直接输出");
        assert_eq!(output, b"plain log\n");
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let mut demux = Demux::new(false);
        let mut input = vec![1, 0, 0, 0];
        input.extend_from_slice(&u32::MAX.to_be_bytes());
        let mut output = Vec::new();
        assert!(demux.feed(&input, &mut output).is_err());
    }

    /// 真机回归：`tail=0` 会被引擎当作「最后 0 行」，必须翻译成 `all`。
    #[test]
    fn zero_tail_means_all_lines() {
        assert_eq!(tail_token(0), "all");
        assert_eq!(tail_token(200), "200");
    }

    #[test]
    fn line_prefixer_injects_tag() {
        let mut prefixer = super::LinePrefixer::new("web-1");
        let result = prefixer.process(b"line 1\nline 2\n");
        assert_eq!(result, b"[web-1] line 1\n[web-1] line 2\n");

        let mut split_prefixer = super::LinePrefixer::new("db");
        let part1 = split_prefixer.process(b"hello ");
        let part2 = split_prefixer.process(b"world\nnext\n");
        assert_eq!(part1, b"[db] hello ");
        assert_eq!(part2, b"world\n[db] next\n");
    }

    #[test]
    fn line_prefixer_with_color_injects_ansi() {
        let mut prefixer = super::LinePrefixer::with_color("api-svc", 0);
        let result = prefixer.process(b"ready\n");
        assert_eq!(result, b"\x1b[36m[api-svc]\x1b[0m ready\n");
    }
}

