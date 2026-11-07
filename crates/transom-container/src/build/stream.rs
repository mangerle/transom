//! 构建上下文打包使用的有界写入器与目录遍历。

use super::ignore::DockerIgnore;
use crate::error::ContainerError;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 流式通道写入适配器：将 tar 字节分块送入有界通道。
pub(super) struct ChannelWriter {
    pub(super) sender: flume::Sender<Result<Vec<u8>, std::io::Error>>,
    pub(super) cancel: Arc<AtomicBool>,
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
pub(super) fn append_dir_recursive<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    base_dir: &Path,
    current_dir: &Path,
    ignore: &DockerIgnore,
    cancel: &Arc<AtomicBool>,
) -> Result<(), ContainerError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ContainerError::Cancelled);
    }
    let mut entries = std::fs::read_dir(current_dir)
        .map_err(|error| ContainerError::File {
            path: current_dir.display().to_string(),
            message: format!("读取上下文子目录失败: {error}"),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ContainerError::File {
            path: current_dir.display().to_string(),
            message: format!("遍历上下文条目失败: {error}"),
        })?;
    // 排序保证 tar 条目顺序稳定，避免缓存键与构建结果随文件系统枚举顺序漂移。
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(ContainerError::Cancelled);
        }
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
            builder
                .append_path_with_name(&path, rel_str.as_ref())
                .map_err(|error| ContainerError::File {
                    path: path.display().to_string(),
                    message: format!("打包符号链接失败: {error}"),
                })?;
        }
    }
    Ok(())
}
