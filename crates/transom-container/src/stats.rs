//! 容器统计长流：HTTP 后端使用 Docker 的 `stream=1`，CLI 后端固定节拍单次采样。

use crate::api::DockerApi;
use crate::cli::CliBackend;
use crate::engine::Engine;
use crate::error::ContainerError;
use crate::http;
use crate::models::StatsResponse;
use bytes::Bytes;
use http_body_util::BodyExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use transom_core::ContainerStats;

const STATS_IDLE_TIMEOUT: Duration = Duration::from_secs(8);
const CANCEL_TICK: Duration = Duration::from_millis(250);
const CLI_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const MAX_STREAM_BUFFER: usize = 8 * 1024 * 1024;

/// 容器统计样本回调。
pub type StatsSink<'a> = dyn FnMut(ContainerStats) + Send + 'a;

/// 持续输出统计样本；HTTP 流断开或 CLI 采样失败时返回错误，由调用方重连。
///
/// # Errors
/// 引擎请求失败、统计流空闲超时、JSON 帧损坏或 CLI 采样失败时返回错误。
pub async fn stream_container_stats(
    engine: &Engine,
    container: &str,
    cancel: &Arc<AtomicBool>,
    sink: &mut StatsSink<'_>,
) -> Result<(), ContainerError> {
    if let Some(cli) = engine.cli_backend_ref() {
        return stream_cli_stats(cli, container, cancel, sink).await;
    }
    stream_http_stats(engine.http()?, container, cancel, sink).await
}

async fn stream_http_stats(
    api: &DockerApi,
    container: &str,
    cancel: &Arc<AtomicBool>,
    sink: &mut StatsSink<'_>,
) -> Result<(), ContainerError> {
    let path = format!(
        "/containers/{}/stats?stream=1&one-shot=false",
        http::percent_encode(container)
    );
    let response = tokio::time::timeout(
        STATS_IDLE_TIMEOUT,
        http::request(
            api.transport_ref(),
            hyper::Method::GET,
            &api.versioned_path(&path),
            Bytes::new(),
            None,
        ),
    )
    .await
    .map_err(|_| ContainerError::Http {
        operation: "连接容器统计流".to_string(),
        message: "连接超时".to_string(),
    })??;
    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        return Err(http::status_error(
            status,
            "读取容器统计流",
            body,
            container,
        ));
    }

    let mut body = std::pin::pin!(response.body);
    let mut parser = StatsStreamParser::new();
    let mut last_data = tokio::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let frame = tokio::select! {
            frame = body.frame() => frame,
            _ = tokio::time::sleep(CANCEL_TICK) => {
                if last_data.elapsed() >= STATS_IDLE_TIMEOUT {
                    return Err(ContainerError::Http {
                        operation: "读取容器统计流".to_string(),
                        message: "统计流空闲超时".to_string(),
                    });
                }
                continue;
            }
        };
        let Some(frame) = frame else {
            return Err(ContainerError::Http {
                operation: "读取容器统计流".to_string(),
                message: "统计流已结束".to_string(),
            });
        };
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "读取容器统计流".to_string(),
            message: error.to_string(),
        })?;
        if let Ok(data) = frame.into_data() {
            last_data = tokio::time::Instant::now();
            parser.feed(&data, sink)?;
        }
    }
}

async fn stream_cli_stats(
    cli: &CliBackend,
    container: &str,
    cancel: &Arc<AtomicBool>,
    sink: &mut StatsSink<'_>,
) -> Result<(), ContainerError> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let stats = tokio::time::timeout(STATS_IDLE_TIMEOUT, cli.container_stats(container))
            .await
            .map_err(|_| ContainerError::Http {
                operation: "CLI 采样容器统计".to_string(),
                message: "采样超时".to_string(),
            })??;
        sink(stats);
        let mut remaining = CLI_SAMPLE_INTERVAL;
        while !remaining.is_zero() {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let step = remaining.min(CANCEL_TICK);
            tokio::time::sleep(step).await;
            remaining = remaining.saturating_sub(step);
        }
    }
}

struct StatsStreamParser {
    buffer: Vec<u8>,
}

impl StatsStreamParser {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(16 * 1024),
        }
    }

    fn feed(&mut self, data: &[u8], sink: &mut StatsSink<'_>) -> Result<(), ContainerError> {
        self.buffer.extend_from_slice(data);
        let mut consumed = 0;
        loop {
            while self
                .buffer
                .get(consumed)
                .is_some_and(u8::is_ascii_whitespace)
            {
                consumed += 1;
            }
            let remaining = &self.buffer[consumed..];
            if remaining.is_empty() {
                break;
            }
            let mut frames =
                serde_json::Deserializer::from_slice(remaining).into_iter::<StatsResponse>();
            match frames.next() {
                Some(Ok(response)) => {
                    consumed += frames.byte_offset();
                    sink(response.to_stats(None));
                }
                Some(Err(error)) if error.is_eof() => break,
                Some(Err(error)) => {
                    return Err(ContainerError::Decode {
                        operation: "解析容器统计流".to_string(),
                        message: error.to_string(),
                    });
                }
                None => break,
            }
        }
        if consumed > 0 {
            self.buffer.drain(..consumed);
        }
        if self.buffer.len() > MAX_STREAM_BUFFER {
            return Err(ContainerError::Decode {
                operation: "解析容器统计流".to_string(),
                message: format!("单帧缓冲超过上限: {} 字节", MAX_STREAM_BUFFER),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(total_cpu: u64) -> String {
        format!(
            r#"{{"cpu_stats":{{"cpu_usage":{{"total_usage":{total_cpu}}},"system_cpu_usage":200000,"online_cpus":2}},"precpu_stats":{{"cpu_usage":{{"total_usage":0}},"system_cpu_usage":100000}},"memory_stats":{{"usage":1024,"limit":2048}}}}"#
        )
    }

    #[test]
    fn parser_handles_concatenated_and_split_frames() {
        let mut parser = StatsStreamParser::new();
        let first = sample(1000);
        let second = sample(2000);
        let mut samples = Vec::with_capacity(2);

        let split = first.len() / 2;
        {
            let mut sink = |stats: ContainerStats| samples.push(stats);
            parser
                .feed(&first.as_bytes()[..split], &mut sink)
                .expect("半帧应等待后续数据");
        }
        assert!(samples.is_empty());
        let payload = format!("{}{}\n", &first[split..], second);
        {
            let mut sink = |stats: ContainerStats| samples.push(stats);
            parser
                .feed(payload.as_bytes(), &mut sink)
                .expect("拼接帧应完整解析");
        }

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].memory_used, 1024);
        assert!(samples[1].cpu_percent > 0.0);
    }
}
