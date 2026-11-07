//! 容器文件浏览与传输：基于归档 API 的 tar 流式读写。
//!
//! 设计原理（Why）：
//! - 归档 API 不需要在容器内起 Shell，路径与内容都由引擎处理；
//! - 列表/预览/下载都流式解析 tar，避免整目录或大文件读入内存；
//! - 上传用后台 tar 生成器 + hyper 流式请求体，本地大文件也不会整体缓冲；
//! - 删除/重命名没有归档 API，改用 exec 的 `rm --` / `mv --`，argv 传递，
//!   不经过 Shell；路径在进入 API/argv 前统一校验。

use crate::error::ContainerError;

/// 列表解析的条目上限：异常 tar 不应让同步线程无限增长内存。
const MAX_LIST_ENTRIES: usize = 10_000;
/// 上传/下载的传输块大小。
const TRANSFER_CHUNK: usize = 64 * 1024;

mod archive;
mod listing;
mod read;
#[cfg(test)]
mod tests;
mod write;

pub(crate) use archive::open_archive;
pub use read::{download, download_directory, list_dir, read_file};
pub use write::{
    chmod, chown, copy, create_dir, remove, rename, upload, upload_directory, write_file_bytes,
};

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
