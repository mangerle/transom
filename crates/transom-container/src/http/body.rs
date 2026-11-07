//! 响应体读取与流式转发。

use super::{BODY_READ_TIMEOUT, RawResponse};
use crate::bridge::BridgeItem;
use crate::error::ContainerError;
use bytes::Bytes;
use http_body_util::BodyExt;

/// 把响应体泵进阻塞侧通道，并返回泵任务。
///
/// 设计原理（Why）：连接驱动任务的生命周期挂在响应守卫上，而归档与镜像进度这类
/// 响应要边收边解析、可能持续数分钟。若调用方只把 `response.body` 搬进自己的任务，
/// `RawResponse` 会在函数返回时析构并中止驱动任务，真机表现为读取中途报
/// 「connection closed before message completed」，且因为取决于服务端发完数据的快慢
/// 而呈偶发。这里要求整体接管响应，守卫与响应体同生同灭。
///
/// 代价：调用方不再能读状态码与响应头，只适用于「已经判定 2xx」之后的流式读取。
pub fn pump_body(
    response: RawResponse,
    sender: flume::Sender<BridgeItem>,
) -> tokio::task::JoinHandle<()> {
    let RawResponse {
        status: _,
        headers: _,
        body,
        _connection,
    } = response;
    // 守卫留在任务内部：解析结束或任务被中止时才释放连接。
    tokio::spawn(async move {
        let _connection = _connection;
        let mut body = std::pin::pin!(body);
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => {
                    let Ok(data) = frame.into_data() else {
                        continue;
                    };
                    if sender.send_async(Ok(data.to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send_async(Err(error.to_string())).await;
                    break;
                }
            }
        }
    })
}
/// 读取响应体前缀，最多保留 `limit` 字节，并返回是否还有被丢弃的数据。
///
/// 设计原理（Why）：必须先按帧累计再判断上限，不能 `collect()` 后才知道大小；
/// 否则远端引擎返回超大响应时会先整段读入内存。达到上限后停止读取，由调用方
/// 根据语义决定是截断展示还是报错；连接守卫会在响应释放时终止驱动。
pub(super) async fn read_body_prefix<B>(
    body: B,
    limit: usize,
) -> Result<(Vec<u8>, bool), ContainerError>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    let deadline = tokio::time::Instant::now() + BODY_READ_TIMEOUT;
    let mut body = std::pin::pin!(body);
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut truncated = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(ContainerError::Decode {
                operation: "读取响应体".to_string(),
                message: "读取响应体超时".to_string(),
            });
        }
        let frame = match tokio::time::timeout(remaining, body.frame()).await {
            Err(_) => {
                return Err(ContainerError::Decode {
                    operation: "读取响应体".to_string(),
                    message: "读取响应体超时".to_string(),
                });
            }
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                return Err(ContainerError::Decode {
                    operation: "读取响应体".to_string(),
                    message: error.to_string(),
                });
            }
            Ok(Some(Ok(frame))) => frame,
        };
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let capacity = limit.saturating_sub(bytes.len());
        if data.len() > capacity {
            bytes.extend_from_slice(&data[..capacity]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&data);
    }
    Ok((bytes, truncated))
}
/// 读取响应体的文本摘要（错误诊断用），超限截断。
pub async fn read_body_text<B>(body: B, limit: usize) -> String
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    match read_body_prefix(body, limit).await {
        Ok((bytes, _)) => String::from_utf8_lossy(&bytes).trim().to_string(),
        Err(error) => format!("读取响应体失败: {error}"),
    }
}
/// 读取完整响应体（用于小体积 JSON 响应）。
///
/// # Errors
/// 响应体读取失败、超时或超过上限时返回 [`ContainerError::Decode`]。
pub async fn read_body_bytes<B>(body: B, limit: usize) -> Result<Bytes, ContainerError>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    let (bytes, truncated) = read_body_prefix(body, limit).await?;
    if truncated {
        return Err(ContainerError::Decode {
            operation: "读取响应体".to_string(),
            message: format!("响应超过上限 {limit} 字节"),
        });
    }
    Ok(Bytes::from(bytes))
}
