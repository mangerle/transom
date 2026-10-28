//! SFTP 上传与下载任务构建与入队辅助逻辑。
//!
//! 设计原理（Why）：
//! - 实现初衷：将本地与远端选区到 TransferTask 的构造以及路径校验提取为独立纯函数；
//! - 核心优势：
//!   1. 面板上传与拖拽上传逻辑统一复用；
//!   2. 路径校验失败时整体拒绝，避免部分入队造成混淆；
//!   3. 方便编写单元测试验证路径映射。

use crate::sftp::join_remote;
use crate::sftp::{TransferMode, TransferTask};
use std::path::PathBuf;
use transom_core::{ChecksumAlgorithm, ConnectionId, TransferDirection, VerifyMode};

/// 把本地路径列表转成上传任务；目录与文件走不同的构造分支。
///
/// 目标连接必须显式传入：调度层不再回退主连接，避免上传落到另一台主机。
pub(super) fn upload_tasks(
    paths: &[PathBuf],
    remote_dir: &str,
    resume_enabled: bool,
    verify: bool,
    verify_mode: VerifyMode,
    algorithm: ChecksumAlgorithm,
    target_connection_id: ConnectionId,
) -> Result<Vec<TransferTask>, String> {
    let mode = if resume_enabled {
        TransferMode::Resume
    } else {
        TransferMode::Overwrite
    };
    let mut tasks = Vec::with_capacity(paths.len());
    for local_path in paths {
        let Some(file_name) = local_path.file_name().and_then(|name| name.to_str()) else {
            return Err(rust_i18n::t!("sftp.upload_file").to_string());
        };
        let remote_path = join_remote(remote_dir, file_name);
        let is_directory = std::fs::symlink_metadata(local_path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        let size = if is_directory {
            None
        } else {
            std::fs::metadata(local_path).map(|meta| meta.len()).ok()
        };
        let mut task = if is_directory {
            TransferTask::new_directory(
                TransferDirection::Upload,
                local_path.clone(),
                remote_path,
                size,
                mode,
            )
        } else {
            TransferTask::new(
                TransferDirection::Upload,
                local_path.clone(),
                remote_path,
                size,
                mode,
            )
        };
        task.target_connection_id = Some(target_connection_id);
        task.verify = verify;
        task.verify_mode = verify_mode;
        task.checksum_algorithm = algorithm;
        tasks.push(task);
    }
    Ok(tasks)
}
