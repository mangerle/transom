//! 上传、写文件以及基于 exec 的删除/重命名/权限操作。
use super::archive::{build_dir_tar_stream, build_tar_stream, read_existing_metadata};
use super::validate_container_path;
use crate::engine::Engine;
use crate::error::ContainerError;
use crate::http;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// 复制容器内文件或目录；通过 exec 调用 `cp -a`，保留权限与符号链接。
///
/// # Errors
/// 路径非法、命令执行失败或退出码非零时返回错误。
pub async fn copy(
    engine: &Engine,
    container: &str,
    from: &str,
    to: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    let api = engine.http()?;
    let from = validate_container_path(from)?;
    let to = validate_container_path(to)?;
    let command = vec![
        "cp".to_string(),
        "-a".to_string(),
        "--".to_string(),
        from.clone(),
        to,
    ];
    let code = api.run_command(container, &command, cancel).await?;
    if code != 0 {
        return Err(ContainerError::File {
            path: from,
            message: format!("cp 退出码: {code}"),
        });
    }
    Ok(())
}

/// 修改容器内路径权限；只接受 3-4 位八进制模式。
///
/// # Errors
/// 路径或模式非法、命令执行失败或退出码非零时返回错误。
pub async fn chmod(
    engine: &Engine,
    container: &str,
    path: &str,
    mode: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    let api = engine.http()?;
    let path = validate_container_path(path)?;
    let mode = mode.trim();
    if !valid_file_mode(mode) {
        return Err(ContainerError::File {
            path,
            message: "权限模式必须是 3-4 位八进制数，例如 755".to_string(),
        });
    }
    let command = vec![
        "chmod".to_string(),
        mode.to_string(),
        "--".to_string(),
        path.clone(),
    ];
    let code = api.run_command(container, &command, cancel).await?;
    if code != 0 {
        return Err(ContainerError::File {
            path,
            message: format!("chmod 退出码: {code}"),
        });
    }
    Ok(())
}

/// 修改容器内路径属主；接受 `user` 或 `user:group`。
///
/// # Errors
/// 路径或属主格式非法、命令执行失败或退出码非零时返回错误。
pub async fn chown(
    engine: &Engine,
    container: &str,
    path: &str,
    owner: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    let api = engine.http()?;
    let path = validate_container_path(path)?;
    let owner = owner.trim();
    if !valid_file_owner(owner) {
        return Err(ContainerError::File {
            path,
            message: "属主必须是 user 或 user:group".to_string(),
        });
    }
    let command = vec![
        "chown".to_string(),
        owner.to_string(),
        "--".to_string(),
        path.clone(),
    ];
    let code = api.run_command(container, &command, cancel).await?;
    if code != 0 {
        return Err(ContainerError::File {
            path,
            message: format!("chown 退出码: {code}"),
        });
    }
    Ok(())
}

/// 权限模式只允许 3-4 位八进制数。
pub(super) fn valid_file_mode(mode: &str) -> bool {
    (3..=4).contains(&mode.len()) && mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
}

/// 属主允许 `user` 或 `user:group`，各段只含常见账户名字符。
pub(super) fn valid_file_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.split(':').count() <= 2
        && owner.split(':').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}
