//! 基于 Docker 兼容 REST 的流式镜像构建。

use super::ignore::DockerIgnore;
use super::stream::{ChannelWriter, append_dir_recursive};
use crate::api::DockerApi;
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::Method;
use hyper::body::Frame;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use transom_core::ContainerBuildSpec;

/// Docker /build 接口返回的单行 JSON 消息结构。
#[derive(Debug, Deserialize)]
struct BuildMessage {
    stream: Option<String>,
    error: Option<String>,
    #[serde(rename = "errorDetail")]
    error_detail: Option<BuildErrorDetail>,
}
#[derive(Debug, Deserialize)]
struct BuildErrorDetail {
    message: Option<String>,
}
/// HTTP 构建流的空闲超时：守护进程长时间不输出任何进度即视为挂死。
const BUILD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// 单行未结束缓冲上限：防止异常流持续输出无换行内容撑爆内存。
const BUILD_LINE_BUFFER_LIMIT: usize = 1024 * 1024;
/// 解析单行 Docker 构建流输出并调用进度回调。
///
/// # Errors
/// 若该行指示构建错误，返回 [`ContainerError::Api`]。
pub fn parse_build_line(
    line: &str,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(), ContainerError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    if let Ok(msg) = serde_json::from_str::<BuildMessage>(trimmed) {
        if let Some(err) = msg.error_detail.and_then(|d| d.message).or(msg.error) {
            return Err(ContainerError::Api {
                operation: "构建镜像".to_string(),
                status: 400,
                body: err,
            });
        }
        if let Some(stream) = msg.stream {
            let clean = stream.trim_end_matches(['\r', '\n']).to_string();
            if !clean.is_empty() {
                progress(clean);
            }
        }
    } else {
        progress(trimmed.to_string());
    }
    Ok(())
}
/// 基于 REST API 执行流式镜像构建。
pub async fn build_image_http(
    api: &DockerApi,
    spec: &ContainerBuildSpec,
    cancel: &Arc<AtomicBool>,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(), ContainerError> {
    let context_path = Path::new(&spec.context_path);
    if !context_path.is_dir() {
        return Err(ContainerError::InvalidPath {
            path: spec.context_path.clone(),
            reason: "构建上下文目录不存在或不是目录".to_string(),
        });
    }

    let ignore = DockerIgnore::from_context(context_path);
    let (sender, receiver) = flume::bounded::<Result<Vec<u8>, std::io::Error>>(8);
    let writer = ChannelWriter {
        sender,
        cancel: Arc::clone(cancel),
    };

    let builder_cancel = Arc::clone(cancel);
    let builder_context = context_path.to_path_buf();
    let pack_task = tokio::task::spawn_blocking(move || -> Result<(), ContainerError> {
        let mut builder = tar::Builder::new(writer);
        builder.mode(tar::HeaderMode::Deterministic);
        append_dir_recursive(
            &mut builder,
            &builder_context,
            &builder_context,
            &ignore,
            &builder_cancel,
        )?;
        builder.finish().map_err(|error| ContainerError::File {
            path: builder_context.display().to_string(),
            message: format!("完成 tar 归档失败: {error}"),
        })?;
        Ok(())
    });

    let stream = futures::stream::unfold(receiver, |receiver| async move {
        receiver
            .recv_async()
            .await
            .ok()
            .map(|item| (item, receiver))
    });
    let body = StreamBody::new(stream.map(|item| match item {
        Ok(bytes) => Ok(Frame::data(Bytes::from(bytes))),
        Err(e) => Err(e),
    }));

    let mut query = Vec::new();
    if let Some(df) = &spec.dockerfile
        && !df.is_empty()
    {
        query.push(format!("dockerfile={}", http::percent_encode(df)));
    }
    for tag in &spec.tags {
        if !tag.is_empty() {
            query.push(format!("t={}", http::percent_encode(tag)));
        }
    }
    if spec.no_cache {
        query.push("nocache=1".to_string());
    }
    if let Some(target) = &spec.target
        && !target.is_empty()
    {
        query.push(format!("target={}", http::percent_encode(target)));
    }
    if let Some(platform) = &spec.platform
        && !platform.is_empty()
    {
        query.push(format!("platform={}", http::percent_encode(platform)));
    }
    if !spec.build_args.is_empty() {
        let json =
            serde_json::to_string(&spec.build_args).map_err(|error| ContainerError::Decode {
                operation: "序列化构建参数".to_string(),
                message: error.to_string(),
            })?;
        query.push(format!("buildargs={}", http::percent_encode(&json)));
    }

    let suffix = if query.is_empty() {
        "/build".to_string()
    } else {
        format!("/build?{}", query.join("&"))
    };
    let path = api.versioned_path(&suffix);

    let response = http::request_stream(
        api.transport_ref(),
        Method::POST,
        &path,
        body,
        Some("application/x-tar"),
    )
    .await?;

    let status = response.status;
    if !status.is_success() {
        let body = http::read_body_text(response.body, 8192).await;
        return Err(http::status_error(
            status,
            "构建镜像",
            body,
            &spec.context_path,
        ));
    }

    let mut body = response.body;
    let mut remainder: Vec<u8> = Vec::with_capacity(8192);
    let mut last_data = tokio::time::Instant::now();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ContainerError::Cancelled);
        }
        let frame = tokio::select! {
            frame = body.frame() => frame,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if last_data.elapsed() >= BUILD_IDLE_TIMEOUT {
                    return Err(ContainerError::Http {
                        operation: "构建镜像".to_string(),
                        message: format!("构建流 {} 秒无输出", BUILD_IDLE_TIMEOUT.as_secs()),
                    });
                }
                continue;
            }
        };
        let Some(frame) = frame else {
            break;
        };
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "构建镜像".to_string(),
            message: error.to_string(),
        })?;
        if let Some(data) = frame.data_ref() {
            last_data = tokio::time::Instant::now();
            remainder.extend_from_slice(data);
            while let Some(pos) = remainder.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = remainder.drain(..=pos).collect();
                parse_build_line(&String::from_utf8_lossy(&line), progress)?;
            }
            if remainder.len() > BUILD_LINE_BUFFER_LIMIT {
                return Err(ContainerError::Decode {
                    operation: "解析构建输出".to_string(),
                    message: format!("单行输出超过上限 {BUILD_LINE_BUFFER_LIMIT} 字节"),
                });
            }
        }
    }

    if !remainder.is_empty() {
        parse_build_line(&String::from_utf8_lossy(&remainder), progress)?;
    }

    let pack_result = pack_task.await.map_err(|error| ContainerError::File {
        path: spec.context_path.clone(),
        message: format!("构建上下文打包任务失败: {error}"),
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    pack_result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_build_line_success() {
        let mut lines = Vec::new();
        let mut progress = |l: String| lines.push(l);

        let json = r#"{"stream":"Step 1/2 : FROM alpine\n"}"#;
        assert!(parse_build_line(json, &mut progress).is_ok());
        assert_eq!(lines, vec!["Step 1/2 : FROM alpine"]);
    }

    #[test]
    fn test_parse_build_line_error() {
        let mut lines = Vec::new();
        let mut progress = |l: String| lines.push(l);

        let json =
            r#"{"errorDetail":{"message":"Dockerfile not found"},"error":"Dockerfile not found"}"#;
        let res = parse_build_line(json, &mut progress);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "容器引擎返回错误，操作: 构建镜像，状态码: 400，响应: Dockerfile not found"
        );
    }
}
