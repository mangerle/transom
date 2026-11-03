//! 镜像构建（Build）实现模块。
//!
//! 设计原理（Why）：
//! - Docker / Podman 引擎通过 REST API `POST /build` 接收上下文目录的 tar 归档并执行构建；
//! - 构建过程可能持续数分钟甚至更久，因此构建请求必须使用长超时，并以 chunked 流式读取响应行；
//! - 每一行响应是携带 `stream` 或 `errorDetail` 的 JSON 对象，流式解析不仅可实时呈现构建进度，
//!   更能在出错时第一时间捕获原因并向界面抛出结构化错误；
//! - CLI 兜底则启动 `docker build` 子进程并接管 stdout/stderr 流。

use crate::api::DockerApi;
use crate::cli::CliBackend;
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::Method;
use serde::Deserialize;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use transom_core::ContainerBuildSpec;

/// 构建操作的最大超时时间（15 分钟）。
const BUILD_TIMEOUT: Duration = Duration::from_secs(900);

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

/// 打包指定目录下的所有文件为 tar 归档内存字节。
///
/// # Errors
/// 若目录不存在、不是目录或打包读取失败，返回 [`ContainerError`]。
pub fn pack_context_tar(context_dir: &Path) -> Result<Vec<u8>, ContainerError> {
    if !context_dir.exists() {
        return Err(ContainerError::InvalidPath {
            path: context_dir.display().to_string(),
            reason: "构建上下文目录不存在".to_string(),
        });
    }
    if !context_dir.is_dir() {
        return Err(ContainerError::InvalidPath {
            path: context_dir.display().to_string(),
            reason: "构建上下文路径不是有效目录".to_string(),
        });
    }

    let mut builder = tar::Builder::new(Vec::new());
    builder.mode(tar::HeaderMode::Deterministic);

    builder
        .append_dir_all(".", context_dir)
        .map_err(|error| ContainerError::File {
            path: context_dir.display().to_string(),
            message: error.to_string(),
        })?;
    builder
        .finish()
        .map_err(|error| ContainerError::File {
            path: context_dir.display().to_string(),
            message: error.to_string(),
        })?;

    builder.into_inner().map_err(|error| ContainerError::File {
        path: context_dir.display().to_string(),
        message: error.to_string(),
    })
}

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

/// 基于 REST API 执行镜像构建。
pub async fn build_image_http(
    api: &DockerApi,
    spec: &ContainerBuildSpec,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(), ContainerError> {
    let context_path = Path::new(&spec.context_path);
    let tar_bytes = pack_context_tar(context_path)?;

    let mut query = Vec::new();
    if let Some(df) = &spec.dockerfile {
        if !df.is_empty() {
            query.push(format!("dockerfile={}", http::percent_encode(df)));
        }
    }
    for tag in &spec.tags {
        if !tag.is_empty() {
            query.push(format!("t={}", http::percent_encode(tag)));
        }
    }
    if spec.no_cache {
        query.push("nocache=1".to_string());
    }

    let suffix = if query.is_empty() {
        "/build".to_string()
    } else {
        format!("/build?{}", query.join("&"))
    };
    let path = api.versioned_path(&suffix);

    let response = http::request_with_timeout(
        api.transport_ref(),
        Method::POST,
        &path,
        Bytes::from(tar_bytes),
        Some("application/x-tar"),
        &[],
        BUILD_TIMEOUT,
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
    let mut remainder = String::new();

    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| ContainerError::Http {
            operation: "构建镜像".to_string(),
            message: error.to_string(),
        })?;
        if let Some(data) = frame.data_ref() {
            let chunk = String::from_utf8_lossy(data);
            remainder.push_str(&chunk);

            while let Some(idx) = remainder.find('\n') {
                let line = remainder[..idx].to_string();
                remainder = remainder[idx + 1..].to_string();
                parse_build_line(&line, progress)?;
            }
        }
    }

    if !remainder.trim().is_empty() {
        parse_build_line(&remainder, progress)?;
    }

    Ok(())
}

/// 基于宿主机 CLI 执行镜像构建。
pub async fn build_image_cli(
    cli: &CliBackend,
    spec: &ContainerBuildSpec,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(), ContainerError> {
    let mut args = vec!["build".to_string()];
    if let Some(df) = &spec.dockerfile {
        if !df.is_empty() {
            args.push("-f".to_string());
            args.push(df.clone());
        }
    }
    for tag in &spec.tags {
        if !tag.is_empty() {
            args.push("-t".to_string());
            args.push(tag.clone());
        }
    }
    if spec.no_cache {
        args.push("--no-cache".to_string());
    }
    args.push(spec.context_path.clone());

    let mut cmd = cli.command(&args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|error| ContainerError::Cli {
        command: format!("docker {}", args.join(" ")),
        message: format!("启动构建子进程失败: {error}"),
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut out_reader = stdout.map(|s| BufReader::new(s).lines());
    let mut err_reader = stderr.map(|s| BufReader::new(s).lines());

    let mut stdout_done = out_reader.is_none();
    let mut stderr_done = err_reader.is_none();

    while !stdout_done || !stderr_done {
        tokio::select! {
            res = async {
                match &mut out_reader {
                    Some(r) => r.next_line().await,
                    None => Ok(None),
                }
            }, if !stdout_done => {
                match res {
                    Ok(Some(line)) => progress(line),
                    Ok(None) => stdout_done = true,
                    Err(e) => {
                        log::warn!("读取构建 stdout 异常: {e}");
                        stdout_done = true;
                    }
                }
            }
            res = async {
                match &mut err_reader {
                    Some(r) => r.next_line().await,
                    None => Ok(None),
                }
            }, if !stderr_done => {
                match res {
                    Ok(Some(line)) => progress(line),
                    Ok(None) => stderr_done = true,
                    Err(e) => {
                        log::warn!("读取构建 stderr 异常: {e}");
                        stderr_done = true;
                    }
                }
            }
        }
    }

    let status = child.wait().await.map_err(|error| ContainerError::Cli {
        command: format!("docker {}", args.join(" ")),
        message: format!("等待构建子进程退出失败: {error}"),
    })?;

    if !status.success() {
        return Err(ContainerError::Cli {
            command: format!("docker {}", args.join(" ")),
            message: format!("构建失败，进程退出状态码: {:?}", status.code()),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_pack_context_tar() {
        let temp = tempdir().expect("创建临时目录失败");
        let df_path = temp.path().join("Dockerfile");
        fs::write(&df_path, "FROM scratch\n").expect("写入文件失败");
        let sub_file = temp.path().join("main.rs");
        fs::write(&sub_file, "fn main() {}\n").expect("写入子文件失败");

        let tar_data = pack_context_tar(temp.path()).expect("打包上下文失败");
        assert!(!tar_data.is_empty(), "tar 数据不应为空");

        let mut archive = tar::Archive::new(&tar_data[..]);
        let entries = archive.entries().expect("读取条目失败");
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.expect("条目有效");
            names.push(entry.path().expect("路径有效").to_string_lossy().into_owned());
        }
        assert!(
            names.iter().any(|n| n.contains("Dockerfile")),
            "必须包含 Dockerfile: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("main.rs")),
            "必须包含 main.rs: {names:?}"
        );
    }

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

        let json = r#"{"errorDetail":{"message":"Dockerfile not found"},"error":"Dockerfile not found"}"#;
        let res = parse_build_line(json, &mut progress);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "容器引擎返回错误，操作: 构建镜像，状态码: 400，响应: Dockerfile not found"
        );
    }
}
