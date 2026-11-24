//! 剪贴板文件流沙箱路径防穿越保护。
//!
//! 设计原理（Why）：RDP 剪贴板文件复制（MS-RDPECLIP）接收来自远端的文件路径。
//! 严禁在校验通过前对任意拼接路径执行文件系统写操作（例如提前 `create_dir_all`），
//! 必须先在纯路径层面逐段过滤非法组件（`..`、根目录、驱动器盘符），再结合基准目录的
//! `canonicalize` 校验，确保目标路径严格落于受控沙箱目录之内，彻底杜绝目录穿越与越权建目录。

use crate::error::RdpError;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// 安全地规范化并校验目标落盘路径，确保不会逃逸出指定的受控沙箱目录。
///
/// # Errors
/// - 当 `file_name` 包含 `..`、盘符前缀、空路径或无法解析出有效相对路径时，返回 [`RdpError::PathTraversal`]；
/// - 当无法创建或规范化受控基准目录时，返回 [`RdpError::SandboxIo`]。
pub fn sanitize_destination_path(base_dir: &Path, file_name: &str) -> Result<PathBuf, RdpError> {
    // 统一将 Windows 反斜杠转为正斜杠，再逐个校验路径组件
    let normalized = file_name.replace('\\', "/");
    let mut safe_relative = PathBuf::new();

    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => {
                // 拦截类似 "C:" 这类在 POSIX 上被视为 Normal 的 Windows 盘符片段
                let part_str = part.to_string_lossy();
                if part_str.contains(':') {
                    return Err(RdpError::PathTraversal {
                        path: PathBuf::from(file_name),
                    });
                }
                safe_relative.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(RdpError::PathTraversal {
                    path: PathBuf::from(file_name),
                });
            }
        }
    }

    if safe_relative.as_os_str().is_empty() {
        return Err(RdpError::PathTraversal {
            path: PathBuf::from(file_name),
        });
    }

    if !base_dir.exists() {
        fs::create_dir_all(base_dir).map_err(|e| RdpError::SandboxIo {
            path: base_dir.to_path_buf(),
            source: e,
        })?;
    }

    let canonical_base = base_dir.canonicalize().map_err(|e| RdpError::SandboxIo {
        path: base_dir.to_path_buf(),
        source: e,
    })?;

    let final_path = canonical_base.join(&safe_relative);
    if !final_path.starts_with(&canonical_base) {
        return Err(RdpError::PathTraversal { path: final_path });
    }

    Ok(final_path)
}
