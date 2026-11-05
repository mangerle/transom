//! 镜像构建（Build）实现模块。
//!
//! # 设计原理（Why）
//! - 流式上下文打包：利用后台线程将上下文文件递归流式写入有界管道，并通过 HTTP/1.1 chunked
//!   流式请求体实时上传，彻底避免整项目（含大构建产物）缓存在内存中导致 OOM；
//! - `.dockerignore` 过滤支持：构建前自动解析上下文根目录下的 `.dockerignore`（默认忽略 `.git` 目录），
//!   在遍历打包过程中跳过无关大文件，极大降低网络传输与磁盘 I/O 开销；
//! - 取消安全性保障：接入全局取消标志 [`Arc<AtomicBool>`]，支持用户在构建任意时刻安全取消传输并中断远端守护进程；
//! - 增强构建参数：支持 `build_args`（ARG 传参）、`target`（多阶段构建目标阶段）以及 `platform`（目标架构）。
//!
//! # 代价与局限（Trade-offs）
//! - chunked 流式请求不向引擎发送 `Content-Length`，但符合 Docker REST 标准规范；
//! - 超大目录的遍历耗时取决于本地磁盘 I/O。

use crate::api::DockerApi;
use crate::cli::CliBackend;
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use futures::StreamExt;
use globset::{Glob, GlobSet, GlobSetBuilder};
use http_body_util::{BodyExt, StreamBody};
use hyper::Method;
use hyper::body::Frame;
use serde::Deserialize;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
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

/// .dockerignore 过滤规则匹配器。
#[derive(Debug)]
pub struct DockerIgnore {
    set: GlobSet,
    has_rules: bool,
}

impl DockerIgnore {
    /// 从构建上下文目录解析 .dockerignore，默认始终忽略 .git 目录。
    pub fn from_context(context_dir: &Path) -> Self {
        let ignore_file = context_dir.join(".dockerignore");
        let mut builder = GlobSetBuilder::new();
        let mut count = 0;

        // 默认始终忽略 .git 目录
        if let Ok(glob) = Glob::new(".git") {
            builder.add(glob);
            count += 1;
        }
        if let Ok(glob) = Glob::new(".git/**") {
            builder.add(glob);
            count += 1;
        }

        if let Ok(content) = std::fs::read_to_string(ignore_file) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let clean = trimmed.trim_start_matches("./");
                if let Ok(glob) = Glob::new(clean) {
                    builder.add(glob);
                    count += 1;
                }
                let dir_pattern = format!("{}/**", clean.trim_end_matches('/'));
                if let Ok(glob) = Glob::new(&dir_pattern) {
                    builder.add(glob);
                    count += 1;
                }
            }
        }

        match builder.build() {
            Ok(set) => Self {
                set,
                has_rules: count > 0,
            },
            Err(e) => {
                log::warn!("编译 .dockerignore glob 集合失败: {e}");
                Self {
                    set: GlobSet::empty(),
                    has_rules: false,
                }
            }
        }
    }

    /// 判定指定相对路径是否已被忽略。
    pub fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        if !self.has_rules {
            return false;
        }
        let clean = rel_path.trim_start_matches('/').trim_start_matches("./");
        if self.set.is_match(clean) {
            return true;
        }
        if is_dir {
            let dir_path = format!("{clean}/");
            if self.set.is_match(&dir_path) {
                return true;
            }
        }
        false
    }
}

/// 流式通道写入适配器：将 tar 字节分块送入有界通道。
struct ChannelWriter {
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
    cancel: Arc<AtomicBool>,
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "构建任务已被取消",
            ));
        }
        self.sender
            .send(Ok(buf.to_vec()))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "通道已断开"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 递归打包目录到 tar 流中，并应用 .dockerignore 过滤规则。
fn append_dir_recursive<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    base_dir: &Path,
    current_dir: &Path,
    ignore: &DockerIgnore,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    let read_dir = std::fs::read_dir(current_dir).map_err(|error| ContainerError::File {
        path: current_dir.display().to_string(),
        message: format!("读取上下文子目录失败: {error}"),
    })?;

    for entry in read_dir {
        if cancel.load(Ordering::Relaxed) {
            return Err(ContainerError::Cancelled);
        }
        let entry = entry.map_err(|error| ContainerError::File {
            path: current_dir.display().to_string(),
            message: format!("遍历上下文条目失败: {error}"),
        })?;
        let path = entry.path();
        let rel_path = path.strip_prefix(base_dir).unwrap_or(&path);
        let rel_str = rel_path.to_string_lossy();
        let file_type = entry.file_type().map_err(|error| ContainerError::File {
            path: path.display().to_string(),
            message: format!("获取文件类型失败: {error}"),
        })?;

        let is_dir = file_type.is_dir();
        if ignore.is_ignored(&rel_str, is_dir) {
            continue;
        }

        if is_dir {
            builder
                .append_dir(rel_str.as_ref(), &path)
                .map_err(|error| ContainerError::File {
                    path: path.display().to_string(),
                    message: format!("打包目录条目失败: {error}"),
                })?;
            append_dir_recursive(builder, base_dir, &path, ignore, cancel)?;
        } else if file_type.is_file() {
            builder
                .append_path_with_name(&path, rel_str.as_ref())
                .map_err(|error| ContainerError::File {
                    path: path.display().to_string(),
                    message: format!("打包文件条目失败: {error}"),
                })?;
        } else if file_type.is_symlink() {
            let _ = builder.append_path_with_name(&path, rel_str.as_ref());
        }
    }
    Ok(())
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
    if let Some(target) = &spec.target {
        if !target.is_empty() {
            query.push(format!("target={}", http::percent_encode(target)));
        }
    }
    if let Some(platform) = &spec.platform {
        if !platform.is_empty() {
            query.push(format!("platform={}", http::percent_encode(platform)));
        }
    }
    if !spec.build_args.is_empty() {
        if let Ok(json) = serde_json::to_string(&spec.build_args) {
            query.push(format!("buildargs={}", http::percent_encode(&json)));
        }
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
    let mut remainder = String::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ContainerError::Cancelled);
        }
        let frame = tokio::select! {
            frame = body.frame() => frame,
            _ = tokio::time::sleep(Duration::from_millis(200)) => continue,
        };
        let Some(frame) = frame else {
            break;
        };
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

    let _ = pack_task.await;
    Ok(())
}

/// 基于宿主机 CLI 执行镜像构建。
pub async fn build_image_cli(
    cli: &CliBackend,
    spec: &ContainerBuildSpec,
    cancel: &Arc<AtomicBool>,
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
    if let Some(target) = &spec.target {
        if !target.is_empty() {
            args.push("--target".to_string());
            args.push(target.clone());
        }
    }
    if let Some(platform) = &spec.platform {
        if !platform.is_empty() {
            args.push("--platform".to_string());
            args.push(platform.clone());
        }
    }
    for (key, val) in &spec.build_args {
        args.push("--build-arg".to_string());
        args.push(format!("{key}={val}"));
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
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err(ContainerError::Cancelled);
        }
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
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
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
    fn test_dockerignore_filtering() {
        let temp = tempdir().expect("创建临时目录失败");
        let root = temp.path();
        fs::write(
            root.join(".dockerignore"),
            "target/\n*.tmp\nignore_me.txt\n",
        )
        .expect("写入 dockerignore 失败");

        let ignore = DockerIgnore::from_context(root);
        assert!(ignore.is_ignored(".git", true));
        assert!(ignore.is_ignored(".git/config", false));
        assert!(ignore.is_ignored("target", true));
        assert!(ignore.is_ignored("target/debug/app", false));
        assert!(ignore.is_ignored("test.tmp", false));
        assert!(ignore.is_ignored("ignore_me.txt", false));
        assert!(!ignore.is_ignored("src/main.rs", false));
        assert!(!ignore.is_ignored("Dockerfile", false));
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
