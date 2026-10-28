//! SFTP 目录传输路径处理与安全校验工具。

use std::path::{Path, PathBuf};
use transom_core::{AppError, ErrorCode};

/// 拼接远端绝对路径与相对子路径。
pub(crate) fn join_remote_path(root: &str, relative: &str) -> String {
    if root == "/" {
        format!("/{relative}")
    } else {
        format!("{}/{}", root.trim_end_matches('/'), relative)
    }
}

/// 将相对路径逐段拼接到本地根目录，并拒绝任何越界片段。
pub(crate) fn local_path_for(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        if !safe_component(component) {
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("传输路径包含非法片段: {relative}"),
            ));
        }
        path.push(component);
    }
    Ok(path)
}

/// 校验单段路径组件是否安全（禁止空段、.、.. 或包含路径分隔符）。
pub(crate) fn safe_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

/// 封装本地 I/O 错误为领域 AppError。
pub(crate) fn local_io_error(context: &str, path: &Path, error: std::io::Error) -> AppError {
    AppError::with_detail(
        ErrorCode::SftpFailed,
        format!("{context} {}: {error}", path.display()),
    )
}
