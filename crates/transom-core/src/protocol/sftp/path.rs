//! SFTP 路径安全校验与 POSIX 文件权限格式化。

use crate::error::{AppError, ErrorCode};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// 判断 SFTP 条目名是否是可安全拼接到本地目录的单个普通路径组件。
///
/// 设计原理（Why）：远端 `readdir` 返回的 `filename` 完全由服务端控制，
/// 恶意或被控服务端可以返回 `"/etc/cron.d/x"`、`"../../.bashrc"` 这类名字。
/// Rust 的 `Path::join` 遇到绝对路径会整体替换基目录，遇到 `..` 会向上逃逸，
/// 因此本地写入前必须把信任边界前移到构造条目处。这里用宿主平台的
/// `Path::components` 判定：Unix 上反斜杠是合法文件名字符，Windows 上则是
/// 分隔符；同一套判断在两端都不会把非单段名字当成可信路径。
pub fn is_safe_file_name(name: &str) -> bool {
    if name.is_empty() || name.contains('\0') {
        return false;
    }
    let mut components = Path::new(name).components();
    let Some(Component::Normal(component)) = components.next() else {
        return false;
    };
    component == OsStr::new(name) && components.next().is_none()
}

/// 把单个远端文件名安全地拼接到本地下载根目录。
///
/// # Errors
/// 文件名为空、`.`、`..`、绝对路径或包含路径分隔符时返回
/// [`ErrorCode::SftpFailed`]，错误信息保留原始文件名便于定位。
pub fn safe_local_join(root: &Path, name: &str) -> Result<PathBuf, AppError> {
    if !is_safe_file_name(name) {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("拒绝非法的远端文件名: {name:?}"),
        ));
    }
    Ok(root.join(name))
}

/// 将 Linux 文件权限掩码格式化为 3 位八进制字符串（如 `755`、`644`）。
///
/// 仅提取低 9 位（Owner/Group/Others 的 rwx 标志），避免文件类型高位干扰 UI 展示与编辑。
pub fn format_octal_permissions(permissions: u32) -> String {
    format!("{:03o}", permissions & 0o777)
}

/// 将 Linux 文件权限掩码格式化为 9 字符 `rwxr-xr-x` 形式。
pub fn format_rwx_permissions(permissions: u32) -> String {
    let mode = permissions & 0o777;
    let flags = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    let mut out = String::with_capacity(9);
    for (mask, ch) in flags {
        out.push(if mode & mask != 0 { ch } else { '-' });
    }
    out
}

/// 解析用户输入的八进制权限字符串（如 `"755"` 或 `"0644"`）为 9 位权限掩码。
pub fn parse_octal_permissions(input: &str) -> Option<u32> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 4 {
        return None;
    }
    let val = u32::from_str_radix(trimmed, 8).ok()?;
    if val <= 0o777 { Some(val) } else { None }
}
