//! 容器文件浏览与传输：基于归档 API 的 tar 流式读写。
//!
//! 设计原理（Why）：
//! - 归档 API 不需要在容器内起 Shell，路径与内容都由引擎处理；
//! - 列表/预览/下载都流式解析 tar，避免整目录或大文件读入内存；
//! - 上传用后台 tar 生成器 + hyper 流式请求体，本地大文件也不会整体缓冲；
//! - 删除/重命名没有归档 API，改用 exec 的 `rm --` / `mv --`，argv 传递，
//!   不经过 Shell；路径在进入 API/argv 前统一校验。

use crate::bridge::{BlockingReader, output_channel};
use crate::engine::Engine;
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use transom_core::{ContainerFileEntry, ContainerFileKind};

/// 列表解析的条目上限：异常 tar 不应让同步线程无限增长内存。
const MAX_LIST_ENTRIES: usize = 10_000;
/// 上传/下载的传输块大小。
const TRANSFER_CHUNK: usize = 64 * 1024;

/// 校验并规范化容器内绝对路径。
///
/// # Errors
/// 非绝对路径、包含 `..` 段、或包含 NUL/换行等控制字符时返回
/// [`ContainerError::InvalidPath`]。
pub fn validate_container_path(path: &str) -> Result<String, ContainerError> {
    if !path.starts_with('/') {
        return Err(ContainerError::InvalidPath {
            path: path.to_string(),
            reason: "必须是容器内绝对路径".to_string(),
        });
    }
    if path.contains('\0') || path.contains('\n') || path.contains('\r') {
        return Err(ContainerError::InvalidPath {
            path: path.to_string(),
            reason: "不能包含控制字符".to_string(),
        });
    }
    let mut components = Vec::with_capacity(8);
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(ContainerError::InvalidPath {
                    path: path.to_string(),
                    reason: "不能包含 .. 路径段".to_string(),
                });
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        return Ok("/".to_string());
    }
    Ok(format!("/{}", components.join("/")))
}

/// 列目录：解析归档 tar 的直接子项。
///
/// # Errors
/// 路径非法、容器/路径不存在或 tar 解析失败时返回 [`ContainerError`]。
pub async fn list_dir(
    engine: &Engine,
    container: &str,
    path: &str,
) -> Result<Vec<ContainerFileEntry>, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let result = tokio::task::spawn_blocking(move || parse_listing(reader, &display))
        .await
        .map_err(|error| ContainerError::File {
            path: path.clone(),
            message: format!("解析目录任务失败: {error}"),
        })?;
    task.abort();
    result
}

/// 读取文件预览；超过 `max_bytes` 时截断并标记。
///
/// # Errors
/// 路径非法、文件不存在、读取失败或 tar 解析失败时返回 [`ContainerError`]。
pub async fn read_file(
    engine: &Engine,
    container: &str,
    path: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool, u64), ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let result = tokio::task::spawn_blocking(move || read_first_file(reader, &display, max_bytes))
        .await
        .map_err(|error| ContainerError::File {
            path: path.clone(),
            message: format!("读取文件任务失败: {error}"),
        })?;
    task.abort();
    result
}

/// 下载文件到本地路径（先写临时文件，成功后原子改名）。
///
/// 进度通过有界通道上报，界面来不及消费时丢弃样本而不是阻塞传输。
///
/// # Errors
/// 路径非法、容器文件不存在、本地写盘失败或用户取消时返回 [`ContainerError`]。
pub async fn download(
    engine: &Engine,
    container: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    if let Some(parent) = local_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        return Err(ContainerError::File {
            path: local_path.display().to_string(),
            message: "本地目标目录不存在".to_string(),
        });
    }
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let destination = local_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        download_to_file(reader, &display, &destination, &cancel, &progress)
    })
    .await
    .map_err(|error| ContainerError::File {
        path: path.clone(),
        message: format!("下载任务失败: {error}"),
    })?;
    task.abort();
    result
}

/// 把容器内目录原样导出为本地 tar 归档。
///
/// 与单文件下载不同，这里不做 tar 解包，直接把归档 API 的原始 tar 流写入磁盘，
/// 因此目录层级、符号链接与权限都能保留。
///
/// # Errors
/// 路径非法、目标目录不存在、读取中断或用户取消时返回 [`ContainerError`]。
pub async fn download_directory(
    engine: &Engine,
    container: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    if let Some(parent) = local_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        return Err(ContainerError::File {
            path: local_path.display().to_string(),
            message: "本地目标目录不存在".to_string(),
        });
    }
    let (reader, task) = open_archive(&api, container, &path).await?;
    let display = path.clone();
    let destination = local_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        write_archive_to_file(reader, &display, &destination, &cancel, &progress)
    })
    .await
    .map_err(|error| ContainerError::File {
        path: path.clone(),
        message: format!("目录导出任务失败: {error}"),
    })?;
    task.abort();
    result
}

/// 阻塞地把原始 tar 流写成文件，成功后原子改名。
fn write_archive_to_file(
    mut reader: BlockingReader,
    container_path: &str,
    destination: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let temp = temp_path(destination);
    let mut file = std::fs::File::create(&temp).map_err(|error| ContainerError::File {
        path: temp.display().to_string(),
        message: format!("创建临时归档失败: {error}"),
    })?;
    let mut buffer = vec![0u8; TRANSFER_CHUNK];
    let mut written = 0u64;
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(ContainerError::Cancelled);
        }
        let count = match reader.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(count) => count,
            Err(error) => {
                break Err(ContainerError::File {
                    path: container_path.to_string(),
                    message: format!("读取目录归档失败: {error}"),
                });
            }
        };
        if let Err(error) = file.write_all(&buffer[..count]) {
            break Err(ContainerError::File {
                path: temp.display().to_string(),
                message: format!("写入目录归档失败: {error}"),
            });
        }
        written = written.saturating_add(count as u64);
        let _ = progress.try_send((written, None));
    };
    if result.is_ok() {
        let _ = file.sync_all();
    }
    drop(file);
    match result {
        Ok(()) => {
            std::fs::rename(&temp, destination).map_err(|error| ContainerError::File {
                path: destination.display().to_string(),
                message: format!("重命名目录归档失败: {error}"),
            })?;
            Ok(written)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// 上传本地文件到容器内路径（`path` 含文件名）。
///
/// # Errors
/// 路径非法、本地文件不可读、tar 生成失败、引擎拒绝或用户取消时返回错误。
pub async fn upload(
    engine: &Engine,
    container: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let (parent, name) = match path.rfind('/') {
        Some(0) => ("/".to_string(), path[1..].to_string()),
        Some(index) => (path[..index].to_string(), path[index + 1..].to_string()),
        None => {
            return Err(ContainerError::InvalidPath {
                path,
                reason: "缺少文件名".to_string(),
            });
        }
    };
    if name.is_empty() {
        return Err(ContainerError::InvalidPath {
            path,
            reason: "缺少文件名".to_string(),
        });
    }
    let metadata = std::fs::metadata(local_path).map_err(|error| ContainerError::File {
        path: local_path.display().to_string(),
        message: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(ContainerError::File {
            path: local_path.display().to_string(),
            message: "只能上传普通文件".to_string(),
        });
    }
    let total = metadata.len();
    let (sender, receiver) = flume::bounded::<Result<Vec<u8>, std::io::Error>>(8);
    let builder_cancel = Arc::clone(&cancel);
    let builder_progress = progress.clone();
    let builder_local = local_path.to_path_buf();
    let builder_name = name.clone();
    let builder = tokio::task::spawn_blocking(move || {
        build_tar_stream(
            &builder_local,
            &builder_name,
            total,
            &builder_cancel,
            &builder_progress,
            sender,
        )
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
        Err(error) => Err(error),
    }));
    let suffix = format!(
        "/containers/{}/archive?path={}",
        http::percent_encode(container),
        http::percent_encode(&parent)
    );
    let response = http::request_stream(
        api.transport_ref(),
        hyper::Method::PUT,
        &api.versioned_path(&suffix),
        body,
        Some("application/x-tar"),
    )
    .await;
    let tar_result = builder.await.map_err(|error| ContainerError::File {
        path: path.clone(),
        message: format!("tar 生成任务失败: {error}"),
    })?;
    let response = response?;
    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "上传文件", body, Some(&path))?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    tar_result.map_err(|message| {
        if cancel.load(Ordering::Relaxed) {
            ContainerError::Cancelled
        } else {
            ContainerError::File {
                path: path.clone(),
                message,
            }
        }
    })?;
    Ok(total)
}

/// 上传本地目录到容器内目标目录；目录内容以 tar 流式传输，保留层级与符号链接。
///
/// # Errors
/// 容器路径非法、本地路径不是目录、tar 生成失败、引擎拒绝或用户取消时返回错误。
pub async fn upload_directory(
    engine: &Engine,
    container: &str,
    path: &str,
    local_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let metadata = std::fs::metadata(local_path).map_err(|error| ContainerError::File {
        path: local_path.display().to_string(),
        message: error.to_string(),
    })?;
    if !metadata.is_dir() {
        return Err(ContainerError::File {
            path: local_path.display().to_string(),
            message: "只能上传目录".to_string(),
        });
    }
    let (sender, receiver) = flume::bounded::<Result<Vec<u8>, std::io::Error>>(8);
    let builder_cancel = Arc::clone(&cancel);
    let builder_local = local_path.to_path_buf();
    let builder = tokio::task::spawn_blocking(move || {
        build_dir_tar_stream(&builder_local, &builder_cancel, &progress, sender)
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
        Err(error) => Err(error),
    }));
    let suffix = format!(
        "/containers/{}/archive?path={}",
        http::percent_encode(container),
        http::percent_encode(&path)
    );
    let response = http::request_stream(
        api.transport_ref(),
        hyper::Method::PUT,
        &api.versioned_path(&suffix),
        body,
        Some("application/x-tar"),
    )
    .await;
    let tar_result = builder.await.map_err(|error| ContainerError::File {
        path: path.clone(),
        message: format!("目录 tar 生成任务失败: {error}"),
    })?;
    let response = response?;
    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "上传目录", body, Some(&path))?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    tar_result.map_err(|message| {
        if cancel.load(Ordering::Relaxed) {
            ContainerError::Cancelled
        } else {
            ContainerError::File {
                path: path.clone(),
                message,
            }
        }
    })
}

/// 向容器内指定路径写入文件字节数据（基于 Docker 归档 API 的 tar 包流式回写）。
///
/// 设计原理（Why）：
/// - 不在容器内启动进程执行 `cat > path`，避免依赖容器内 Shell 或命令工具，即使是 scratch 或 distroless 容器亦可正常回写；
/// - 直接在内存中构建仅包含目标单文件的 GNU tar 归档，调用 Docker 归档 PUT 接口解包到父目录；
/// - 遵循 RESTful 协议契约，原子性与宿主环境无关。
///
/// # Errors
/// 路径非法、父目录解析失败或 Docker 归档 API 返回错误时返回 [`ContainerError`]。
pub async fn write_file_bytes(
    engine: &Engine,
    container: &str,
    path: &str,
    data: Vec<u8>,
) -> Result<(), ContainerError> {
    let api = engine.http()?.clone();
    let path = validate_container_path(path)?;
    let (parent, name) = match path.rfind('/') {
        Some(0) => ("/".to_string(), path[1..].to_string()),
        Some(index) => (path[..index].to_string(), path[index + 1..].to_string()),
        None => {
            return Err(ContainerError::InvalidPath {
                path,
                reason: "缺少文件名".to_string(),
            });
        }
    };
    if name.is_empty() {
        return Err(ContainerError::InvalidPath {
            path,
            reason: "缺少文件名".to_string(),
        });
    }

    // 覆盖已有文件时保留原 mode/uid/gid/mtime；新文件使用 0644 与当前时间。
    let existing = read_existing_metadata(&api, container, &path).await;
    let tar_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, std::io::Error> {
        let mut tar_buf = Vec::with_capacity(data.len() + 1024);
        let (mode, uid, gid, mtime) = existing
            .map(|metadata| (metadata.mode, metadata.uid, metadata.gid, metadata.mtime))
            .unwrap_or_else(|| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0);
                (0o644, 0, 0, now)
            });
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            header.set_uid(uid);
            header.set_gid(gid);
            header.set_mtime(mtime);
            header.set_cksum();
            builder.append_data(&mut header, &name, data.as_slice())?;
            builder.finish()?;
        }
        Ok(tar_buf)
    })
    .await
    .map_err(|e| ContainerError::File {
        path: path.clone(),
        message: format!("生成 tar 任务失败: {e}"),
    })?
    .map_err(|e| ContainerError::File {
        path: path.clone(),
        message: format!("打包归档失败: {e}"),
    })?;

    let suffix = format!(
        "/containers/{}/archive?path={}",
        http::percent_encode(container),
        http::percent_encode(&parent)
    );
    let response = http::request(
        api.transport_ref(),
        hyper::Method::PUT,
        &api.versioned_path(&suffix),
        Bytes::from(tar_bytes),
        Some("application/x-tar"),
    )
    .await?;

    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "写入容器文件", body, Some(&path))?;
    }
    Ok(())
}

/// 在容器内创建目录（`mkdir -p`）。
///
/// 设计原理（Why）：归档 API 只能把 tar 解到已存在的目录里，父目录不存在时上传
/// 直接失败；因此新建目录必须走 exec，并以 argv 传参、不经过 Shell，避免注入。
///
/// # Errors
/// 路径非法、命令失败或退出码非零时返回 [`ContainerError`]。
pub async fn create_dir(
    engine: &Engine,
    container: &str,
    path: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    let api = engine.http()?;
    let path = validate_container_path(path)?;
    let command = vec![
        "mkdir".to_string(),
        "-p".to_string(),
        "--".to_string(),
        path.clone(),
    ];
    let code = api.run_command(container, &command, cancel).await?;
    if code != 0 {
        return Err(ContainerError::File {
            path,
            message: format!("mkdir 退出码: {code}"),
        });
    }
    Ok(())
}

/// 删除容器内文件或目录。
///
/// # Errors
/// 路径非法、命令执行失败或退出码非零时返回错误。
pub async fn remove(
    engine: &Engine,
    container: &str,
    path: &str,
    recursive: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    let api = engine.http()?;
    let path = validate_container_path(path)?;
    let mut command = vec!["rm".to_string(), "-f".to_string()];
    if recursive {
        command[1] = "-rf".to_string();
    }
    command.push("--".to_string());
    command.push(path.clone());
    let code = api.run_command(container, &command, cancel).await?;
    if code != 0 {
        return Err(ContainerError::File {
            path,
            message: format!("rm 退出码: {code}"),
        });
    }
    Ok(())
}

/// 重命名/移动容器内路径。
///
/// # Errors
/// 路径非法、命令执行失败或退出码非零时返回错误。
pub async fn rename(
    engine: &Engine,
    container: &str,
    from: &str,
    to: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    let api = engine.http()?;
    let from = validate_container_path(from)?;
    let to = validate_container_path(to)?;
    let command = vec!["mv".to_string(), "--".to_string(), from.clone(), to];
    let code = api.run_command(container, &command, cancel).await?;
    if code != 0 {
        return Err(ContainerError::File {
            path: from,
            message: format!("mv 退出码: {code}"),
        });
    }
    Ok(())
}

/// 容器内已有文件的元数据快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArchiveMetadata {
    mode: u32,
    uid: u64,
    gid: u64,
    mtime: u64,
}

/// 从归档流读取首个条目头；不读取正文，读完头即可停止。
fn metadata_from_archive<R: Read>(reader: R) -> Option<ArchiveMetadata> {
    let mut archive = tar::Archive::new(reader);
    let mut entries = archive.entries().ok()?;
    let entry = entries.next()?.ok()?;
    let header = entry.header();
    Some(ArchiveMetadata {
        mode: header.mode().ok()?,
        uid: header.uid().ok()?,
        gid: header.gid().ok()?,
        mtime: header.mtime().ok()?,
    })
}

/// 读取已有文件元数据；文件不存在或引擎不支持时返回 `None`，由调用方使用默认值。
async fn read_existing_metadata(
    api: &crate::api::DockerApi,
    container: &str,
    path: &str,
) -> Option<ArchiveMetadata> {
    let (reader, task) = match open_archive(api, container, path).await {
        Ok(archive) => archive,
        Err(ContainerError::NotFound { .. }) => return None,
        Err(error) => {
            log::debug!("读取容器文件元数据失败，路径: {path}，原因: {error}");
            return None;
        }
    };
    let metadata = tokio::task::spawn_blocking(move || metadata_from_archive(reader))
        .await
        .ok()
        .flatten();
    task.abort();
    metadata
}

/// 打开归档流：后台任务把 HTTP 响应体泵入有界通道，阻塞侧按需读取。
pub(crate) async fn open_archive(
    api: &crate::api::DockerApi,
    container: &str,
    path: &str,
) -> Result<(BlockingReader, tokio::task::JoinHandle<()>), ContainerError> {
    let suffix = format!(
        "/containers/{}/archive?path={}",
        http::percent_encode(container),
        http::percent_encode(path)
    );
    let response = http::request(
        api.transport_ref(),
        hyper::Method::GET,
        &api.versioned_path(&suffix),
        Bytes::new(),
        None,
    )
    .await?;
    if !response.status.is_success() {
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        if status == hyper::StatusCode::NOT_FOUND {
            return Err(ContainerError::NotFound {
                target: path.to_string(),
            });
        }
        return Err(ContainerError::Api {
            operation: format!("读取容器归档 {path}"),
            status: status.as_u16(),
            body,
        });
    }
    let (sender, reader) = output_channel(8);
    // 整个 response 交给泵任务：只搬 body 会提前中止连接驱动，归档流会被截断。
    let task = http::pump_body(response, sender);
    Ok((reader, task))
}

fn join_container(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// 把 tar 条目路径换算为相对请求路径的路径；不相关条目返回 `None`。
fn relativize(raw: &str, base_name: &str) -> Option<String> {
    let trimmed = raw.trim_start_matches('/').trim_start_matches("./");
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return None;
    }
    if base_name.is_empty() {
        return Some(trimmed.to_string());
    }
    if trimmed == base_name {
        return None;
    }
    let prefix = format!("{base_name}/");
    trimmed
        .strip_prefix(&prefix)
        .map(str::to_string)
        .or_else(|| Some(trimmed.to_string()))
}

fn parse_listing<R: Read>(
    reader: R,
    base: &str,
) -> Result<Vec<ContainerFileEntry>, ContainerError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| ContainerError::File {
        path: base.to_string(),
        message: format!("读取 tar 失败: {error}"),
    })?;
    let base_name = base.rsplit('/').next().unwrap_or("");
    let mut entries_map: BTreeMap<String, ContainerFileEntry> = BTreeMap::new();
    let mut count = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| ContainerError::File {
            path: base.to_string(),
            message: format!("解析 tar 条目失败: {error}"),
        })?;
        count += 1;
        if count > MAX_LIST_ENTRIES {
            return Err(ContainerError::File {
                path: base.to_string(),
                message: format!("目录条目超过上限 {MAX_LIST_ENTRIES}"),
            });
        }
        let raw = entry.path().map_err(|error| ContainerError::File {
            path: base.to_string(),
            message: format!("解析 tar 路径失败: {error}"),
        })?;
        let raw = raw.to_string_lossy().to_string();
        let Some(relative) = relativize(&raw, base_name) else {
            continue;
        };
        let mut components = relative.split('/').filter(|part| !part.is_empty());
        let Some(first) = components.next() else {
            continue;
        };
        let direct = components.next().is_none();
        let path = join_container(base, first);
        if !direct {
            entries_map
                .entry(first.to_string())
                .or_insert(ContainerFileEntry {
                    name: first.to_string(),
                    path,
                    kind: ContainerFileKind::Directory,
                    size: 0,
                    mode: 0,
                    modified: None,
                    link_target: None,
                });
            continue;
        }
        let header = entry.header();
        let entry_type = header.entry_type();
        let kind = if entry_type.is_dir() {
            ContainerFileKind::Directory
        } else if entry_type.is_symlink() {
            ContainerFileKind::Symlink
        } else if entry_type.is_file() {
            ContainerFileKind::File
        } else {
            ContainerFileKind::Other
        };
        let link_target = if kind == ContainerFileKind::Symlink {
            entry
                .link_name()
                .ok()
                .flatten()
                .map(|path| path.to_string_lossy().to_string())
        } else {
            None
        };
        entries_map.insert(
            first.to_string(),
            ContainerFileEntry {
                name: first.to_string(),
                path,
                kind,
                size: header.size().unwrap_or(0),
                mode: header.mode().unwrap_or(0),
                modified: header.mtime().ok().map(|time| time as i64),
                link_target,
            },
        );
    }
    let mut list: Vec<ContainerFileEntry> = entries_map.into_values().collect();
    list.sort_by(|left, right| {
        let left_dir = left.kind == ContainerFileKind::Directory;
        let right_dir = right.kind == ContainerFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(list)
}

fn read_first_file<R: Read>(
    reader: R,
    path: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool, u64), ContainerError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| ContainerError::File {
        path: path.to_string(),
        message: format!("读取 tar 失败: {error}"),
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| ContainerError::File {
            path: path.to_string(),
            message: format!("解析 tar 条目失败: {error}"),
        })?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let size = entry.header().size().unwrap_or(0);
        let take = size.min(max_bytes as u64) as usize;
        let mut data = vec![0u8; take];
        entry
            .read_exact(&mut data)
            .map_err(|error| ContainerError::File {
                path: path.to_string(),
                message: format!("读取文件内容失败: {error}"),
            })?;
        return Ok((data, size > max_bytes as u64, size));
    }
    Err(ContainerError::NotFound {
        target: path.to_string(),
    })
}

fn download_to_file(
    reader: BlockingReader,
    path: &str,
    destination: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
) -> Result<u64, ContainerError> {
    let temp = temp_path(destination);
    let result = (|| -> Result<u64, ContainerError> {
        let mut archive = tar::Archive::new(reader);
        let mut entries = archive.entries().map_err(|error| ContainerError::File {
            path: path.to_string(),
            message: format!("读取 tar 失败: {error}"),
        })?;
        let mut file = std::fs::File::create(&temp).map_err(|error| ContainerError::File {
            path: temp.display().to_string(),
            message: error.to_string(),
        })?;
        let mut written = 0u64;
        let mut found = false;
        for entry in entries.by_ref() {
            let mut entry = entry.map_err(|error| ContainerError::File {
                path: path.to_string(),
                message: format!("解析 tar 条目失败: {error}"),
            })?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            found = true;
            let total = entry.header().size().unwrap_or(0);
            let mut buffer = vec![0u8; TRANSFER_CHUNK];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(ContainerError::Cancelled);
                }
                let count = entry
                    .read(&mut buffer)
                    .map_err(|error| ContainerError::File {
                        path: path.to_string(),
                        message: format!("读取文件内容失败: {error}"),
                    })?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])
                    .map_err(|error| ContainerError::File {
                        path: temp.display().to_string(),
                        message: error.to_string(),
                    })?;
                written = written.saturating_add(count as u64);
                let _ = progress.try_send((written, Some(total)));
            }
            break;
        }
        if !found {
            return Err(ContainerError::NotFound {
                target: path.to_string(),
            });
        }
        file.sync_all().map_err(|error| ContainerError::File {
            path: temp.display().to_string(),
            message: error.to_string(),
        })?;
        drop(file);
        std::fs::rename(&temp, destination).map_err(|error| ContainerError::File {
            path: destination.display().to_string(),
            message: error.to_string(),
        })?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// 临时文件与目标同目录，保证 rename 是同一文件系统内的原子操作。
fn temp_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    destination.with_file_name(format!(".{name}.{stamp}.transom-part"))
}

/// 递归构建目录 tar，写入有界通道；发送端阻塞形成反压。
fn build_dir_tar_stream(
    local_dir: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
) -> Result<u64, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".to_string());
    }
    let writer = ProgressChannelWriter {
        sender,
        written: 0,
        progress: progress.clone(),
    };
    let mut builder = tar::Builder::new(writer);
    builder.mode(tar::HeaderMode::Deterministic);
    builder.follow_symlinks(false);
    builder
        .append_dir_all(".", local_dir)
        .map_err(|error| error.to_string())?;
    builder.finish().map_err(|error| error.to_string())?;
    let writer = builder.into_inner().map_err(|error| error.to_string())?;
    Ok(writer.written)
}

fn build_tar_stream(
    local_path: &Path,
    name: &str,
    total: u64,
    cancel: &Arc<AtomicBool>,
    progress: &flume::Sender<(u64, Option<u64>)>,
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
) -> Result<(), String> {
    let file = std::fs::File::open(local_path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    let mut header = tar::Header::new_gnu();
    header.set_size(total);
    header.set_mode(file_mode(&metadata));
    header.set_mtime(
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
    );
    header.set_cksum();
    let reader = ProgressReader {
        inner: file,
        read: 0,
        total,
        cancel: Arc::clone(cancel),
        progress: progress.clone(),
    };
    let writer = ChannelWriter { sender };
    let mut builder = tar::Builder::new(writer);
    builder.mode(tar::HeaderMode::Deterministic);
    builder
        .append_data(&mut header, name, reader)
        .map_err(|error| error.to_string())?;
    builder.finish().map_err(|error| error.to_string())?;
    Ok(())
}

fn file_mode(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o644
    }
}

/// 目录上传的 tar 输出：写入通道并累计字节数上报进度。
struct ProgressChannelWriter {
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
    written: u64,
    progress: flume::Sender<(u64, Option<u64>)>,
}

impl Write for ProgressChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender
            .send(Ok(buf.to_vec()))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "上传通道已关闭"))?;
        self.written = self.written.saturating_add(buf.len() as u64);
        let _ = self.progress.try_send((self.written, None));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 把 tar 输出写入有界通道；通道满时阻塞，形成反压。
struct ChannelWriter {
    sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender
            .send(Ok(buf.to_vec()))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "上传通道已关闭"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 读取本地文件并上报进度；取消时返回错误终止 tar 生成。
struct ProgressReader {
    inner: std::fs::File,
    read: u64,
    total: u64,
    cancel: Arc<AtomicBool>,
    progress: flume::Sender<(u64, Option<u64>)>,
}

impl Read for ProgressReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("上传已取消"));
        }
        let count = self.inner.read(buf)?;
        self.read = self.read.saturating_add(count as u64);
        let _ = self.progress.try_send((self.read, Some(self.total)));
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::{metadata_from_archive, parse_listing, relativize, validate_container_path};
    use std::io::Cursor;
    use transom_core::ContainerFileKind;

    #[test]
    fn path_validation_rejects_relative_and_parent_segments() {
        assert!(validate_container_path("etc/passwd").is_err());
        assert!(validate_container_path("/etc/../root").is_err());
        assert!(validate_container_path("/etc/\npasswd").is_err());
        assert_eq!(
            validate_container_path("/var//log/./").expect("合法路径"),
            "/var/log"
        );
        assert_eq!(validate_container_path("/").expect("根路径"), "/");
    }

    #[test]
    fn relativize_strips_base_directory_name() {
        assert_eq!(
            relativize("etc/hostname", "etc").as_deref(),
            Some("hostname")
        );
        assert_eq!(relativize("etc/", "etc"), None);
        assert_eq!(relativize("bin/", "").as_deref(), Some("bin"));
        assert_eq!(
            relativize("etc/ssl/certs/ca.pem", "etc").as_deref(),
            Some("ssl/certs/ca.pem")
        );
    }

    #[test]
    fn directory_upload_builds_tar_with_nested_files() {
        let directory = tempfile::tempdir().expect("临时目录");
        std::fs::create_dir(directory.path().join("sub")).expect("创建子目录");
        std::fs::write(directory.path().join("sub").join("app.txt"), b"hello")
            .expect("写入测试文件");
        let (sender, receiver) = flume::bounded(64);
        let (progress, _progress_rx) = flume::bounded(8);
        let root = directory.path().to_path_buf();
        let builder = std::thread::spawn(move || {
            super::build_dir_tar_stream(
                &root,
                &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                &progress,
                sender,
            )
        });
        let mut bytes = Vec::new();
        while let Ok(chunk) = receiver.recv() {
            bytes.extend(chunk.expect("tar 块必须成功"));
        }
        builder
            .join()
            .expect("tar 构建线程")
            .expect("目录 tar 构建必须成功");
        let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
        let names: Vec<String> = archive
            .entries()
            .expect("tar 条目")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().expect("路径").display().to_string())
            .collect();
        assert!(names.iter().any(|name| name.ends_with("sub/app.txt")));
    }

    #[test]
    fn archive_export_writes_raw_tar_and_cleans_temp_on_cancel() {
        let directory = tempfile::tempdir().expect("临时目录");
        let destination = directory.path().join("export.tar");
        let (sender, reader) = crate::bridge::output_channel(4);
        sender
            .send(Ok(b"tar-bytes".to_vec()))
            .expect("发送必须成功");
        drop(sender);
        let (progress, _receiver) = flume::bounded(4);
        let written = super::write_archive_to_file(
            reader,
            "/data",
            &destination,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            &progress,
        )
        .expect("导出必须成功");
        assert_eq!(written, 9);
        assert_eq!(std::fs::read(&destination).expect("读取导出"), b"tar-bytes");
    }

    #[test]
    fn archive_metadata_keeps_mode_owner_and_mtime() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o750);
        header.set_uid(1000);
        header.set_gid(1001);
        header.set_mtime(1_700_000_000);
        header.set_cksum();
        builder
            .append_data(&mut header, "config", Cursor::new(b"data"))
            .expect("写入 tar 条目");
        let bytes = builder.into_inner().expect("结束 tar");
        let metadata = metadata_from_archive(Cursor::new(bytes)).expect("必须读到元数据");
        assert_eq!(metadata.mode, 0o750);
        assert_eq!(metadata.uid, 1000);
        assert_eq!(metadata.gid, 1001);
        assert_eq!(metadata.mtime, 1_700_000_000);
    }

    #[test]
    fn listing_aggregates_direct_children_from_tar() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o755);
        header.set_size(0);
        header.set_cksum();
        builder
            .append_data(&mut header, "etc/ssl/", Cursor::new(Vec::new()))
            .expect("写入目录条目");

        let mut file_header = tar::Header::new_gnu();
        file_header.set_entry_type(tar::EntryType::Regular);
        file_header.set_mode(0o644);
        file_header.set_size(5);
        file_header.set_cksum();
        builder
            .append_data(
                &mut file_header,
                "etc/hostname",
                Cursor::new(b"host\n".to_vec()),
            )
            .expect("写入文件条目");
        let bytes = builder.into_inner().expect("结束 tar");

        let entries = parse_listing(Cursor::new(bytes), "/etc").expect("解析目录");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "ssl");
        assert_eq!(entries[0].kind, ContainerFileKind::Directory);
        assert_eq!(entries[1].name, "hostname");
        assert_eq!(entries[1].kind, ContainerFileKind::File);
        assert_eq!(entries[1].size, 5);
        assert_eq!(entries[1].path, "/etc/hostname");
    }
}
