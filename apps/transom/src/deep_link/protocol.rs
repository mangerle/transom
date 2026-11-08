//! 深链转发的平台无关线协议。
//!
//! 设计原理（Why）：Unix socket 与 Windows named pipe 只负责搬运字节，请求
//! 上限、逐行读取、类型校验和事件投递必须共用一份实现，避免两端安全边界
//! 漂移。平台传输层只需处理连接建立、超时与进程生命周期。

use super::types::{DIR_LINE_PREFIX, EXEC_LINE_PREFIX, LaunchRequest};
use anyhow::{Context as _, anyhow, bail};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use transom_core::{AppEvent, DeepLinkRequest, is_ssh_url};

/// 单条连接允许读取的最大请求字节数。
pub(crate) const MAX_REQUEST_BYTES: u64 = 16 * 1024;
/// 单次请求读取或写入的最长等待时间。
pub(crate) const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// 把启动意图逐行写入平台通道；没有请求时写空行表示“仅唤起”。
pub(crate) fn write_request<W: Write>(writer: &mut W, request: &LaunchRequest) -> io::Result<()> {
    let lines = super::types::request_lines(request);
    for line in &lines {
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    if lines.is_empty() {
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

/// 读完一条连接上的所有请求并逐条投递。
pub(crate) async fn accept_stream<R: AsyncRead + Unpin>(
    stream: &mut R,
    event_tx: &flume::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES));
    loop {
        let mut line = String::new();
        let read = tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| anyhow!("等待深链请求超时，已丢弃在途请求"))?
            .context("读取深链请求失败")?;
        if read == 0 {
            return Ok(());
        }
        let request = line.trim_end_matches(['\n', '\r']).trim();
        accept_request(request, event_tx)?;
    }
}

fn accept_request(request: &str, event_tx: &flume::Sender<AppEvent>) -> anyhow::Result<()> {
    let request = parse_request_line(request)?;
    log::info!("收到外部唤起请求: {request:?}");
    if event_tx.try_send(AppEvent::DeepLink { request }).is_err() {
        log::warn!("深链唤起未能投递到界面事件通道（队列已满），本次请求已丢弃");
    }
    Ok(())
}

fn parse_request_line(request: &str) -> anyhow::Result<DeepLinkRequest> {
    if request.is_empty() {
        return Ok(DeepLinkRequest::Activate);
    }
    if is_ssh_url(request) {
        return Ok(DeepLinkRequest::Ssh {
            url: request.to_string(),
        });
    }
    if let Some(payload) = request.strip_prefix(EXEC_LINE_PREFIX) {
        return parse_exec_line(payload);
    }
    if let Some(path) = request.strip_prefix(DIR_LINE_PREFIX) {
        let candidate = Path::new(path);
        if candidate.is_absolute() && candidate.is_dir() {
            return Ok(DeepLinkRequest::Dir {
                path: candidate.to_path_buf(),
            });
        }
        return Err(anyhow!(
            "目录请求不是现存绝对路径，已拒绝，原始输入: {request}"
        ));
    }
    Err(anyhow!("深链请求协议形态未知，已拒绝，原始输入: {request}"))
}

fn parse_exec_line(payload: &str) -> anyhow::Result<DeepLinkRequest> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .with_context(|| format!("命令请求载荷不是合法 JSON，原始输入: {payload}"))?;
    let command: Vec<String> = value
        .get("command")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if command.is_empty() {
        bail!("命令请求缺少可执行命令，原始输入: {payload}");
    }
    let dir = value
        .get("dir")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .filter(|candidate| candidate.is_absolute() && candidate.is_dir());
    Ok(DeepLinkRequest::Exec { command, dir })
}
