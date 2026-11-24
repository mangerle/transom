//! 剪贴板文件流式传输与沙箱路径防穿越保护。
//!
//! 设计原理（Why）：RDP 剪贴板文件复制（MS-RDPECLIP）通过文件流分块读取与写入。
//! 严禁未经校验直接按对端下发的文件名写入本地文件系统；必须通过 Canonical Path
//! 校验确保目标路径严格位于受控的本地下载目录沙箱之内，杜绝 `../` 越权逃逸。

use crate::error::RdpError;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 文件流分块大小（默认 512KB）。
pub const FILE_CHUNK_SIZE: usize = 512 * 1024;

/// 文件传输阶段状态事件。
#[derive(Debug, Clone)]
pub enum FileTransferEvent {
    /// 开始传输指定文件。
    Started { name: String, total_bytes: u64 },
    /// 文件传输进度更新。
    Progress {
        name: String,
        transferred: u64,
        total: u64,
    },
    /// 文件传输顺利完成。
    Completed { name: String },
    /// 传输失败。
    Failed { name: String, error: String },
}

/// 安全地规范化并校验目标落盘路径，确保不会逃逸出指定的受控沙箱目录。
pub fn sanitize_destination_path(base_dir: &Path, file_name: &str) -> Result<PathBuf, RdpError> {
    // 过滤掉任何可能携带的绝对路径前缀或 Windows 驱动器盘符（如 `C:\`）
    let clean_name = file_name
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .replace('\\', "/");

    let relative_path = Path::new(&clean_name);
    let target = base_dir.join(relative_path);

    // 若父目录存在则先规范化基准目录；不存在时创建
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

    // 如果目标文件尚不存在，校验其父目录归属
    let target_parent = target.parent().unwrap_or(base_dir);
    if !target_parent.exists() {
        fs::create_dir_all(target_parent).map_err(|e| RdpError::SandboxIo {
            path: target_parent.to_path_buf(),
            source: e,
        })?;
    }

    let canonical_parent = target_parent
        .canonicalize()
        .map_err(|e| RdpError::SandboxIo {
            path: target_parent.to_path_buf(),
            source: e,
        })?;

    if !canonical_parent.starts_with(&canonical_base) {
        return Err(RdpError::PathTraversal {
            path: target.to_path_buf(),
        });
    }

    Ok(canonical_parent.join(target.file_name().unwrap_or_default()))
}

/// 读取本地文件指定偏移量的分块数据。
pub fn read_file_chunk(
    path: &Path,
    offset: u64,
    buffer: &mut [u8],
) -> Result<usize, std::io::Error> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    file.read(buffer)
}

/// 写入分块数据到目标文件。
pub fn write_file_chunk(file: &mut File, offset: u64, data: &[u8]) -> Result<(), std::io::Error> {
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)
}
