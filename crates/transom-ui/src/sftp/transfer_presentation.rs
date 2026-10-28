//! 传输任务的展示口径：状态文案、进度百分比、显示名与忙碌判定。
//!
//! 设计原理（Why）：面板队列行与终端右下角浮层是两个布局完全不同的载体，
//! 但"同一个任务"的状态文案、进度与名称必须一致；此前两处各写一份 match，
//! 暂停/完成/取消在队列里是硬编码中文、在浮层里走翻译键，失败前缀也不一致。
//! 这里只收敛与布局无关的纯计算，图标、颜色与按钮由各自视图决定。

use super::{TransferStatus, TransferTask};
use transom_core::TransferDirection;

/// 传输进度百分比（0..=100）；总大小未知或为零时返回 `None`。
///
/// 用饱和钳制而不是直接除法：部分协议在收尾阶段报告 `transferred > size`，
/// 进度条与文案都不该出现 101%。
pub(crate) fn progress_percent(task: &TransferTask) -> Option<f32> {
    task.size
        .filter(|size| *size > 0)
        .map(|size| (task.transferred as f32 * 100.0 / size as f32).clamp(0.0, 100.0))
}

/// 传输状态文案；`Transferring` 只返回动作名，百分比由调用方按各自布局拼接。
pub(crate) fn status_label(status: &TransferStatus) -> String {
    match status {
        TransferStatus::Pending => rust_i18n::t!("sftp.pending").to_string(),
        TransferStatus::Transferring => rust_i18n::t!("sftp.transferring").to_string(),
        TransferStatus::Verifying => rust_i18n::t!("sftp.verifying").to_string(),
        TransferStatus::Paused => rust_i18n::t!("sftp.paused").to_string(),
        TransferStatus::Completed => rust_i18n::t!("sftp.completed").to_string(),
        TransferStatus::Cancelled => rust_i18n::t!("sftp.cancelled").to_string(),
        TransferStatus::Failed(reason) => {
            format!("{}: {reason}", rust_i18n::t!("sftp.failed"))
        }
    }
}

/// 任务在界面上的显示名。
///
/// 跨远端互传优先展示"源名 → 目标名"（同名时只显示一次），其余任务取本地
/// 文件名，本地路径缺失时回退远端路径，保证任何任务都有可读名称。
pub(crate) fn display_name(task: &TransferTask) -> String {
    if task.direction == TransferDirection::RemoteCopy {
        let Some(target) = &task.target_remote_path else {
            return task.remote_path.clone();
        };
        let src_name = task
            .remote_path
            .rsplit('/')
            .next()
            .unwrap_or(&task.remote_path);
        let dst_name = target.rsplit('/').next().unwrap_or(target);
        if src_name == dst_name {
            src_name.to_string()
        } else {
            format!("{src_name} → {dst_name}")
        }
    } else {
        task.local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| task.remote_path.clone())
    }
}

/// 任务是否仍在队列生命周期内（占用并发名额）。
pub(crate) fn is_busy(task: &TransferTask) -> bool {
    matches!(
        task.status,
        TransferStatus::Pending
            | TransferStatus::Transferring
            | TransferStatus::Verifying
            | TransferStatus::Paused
    )
}

#[cfg(test)]
mod tests {
    use super::{display_name, is_busy, progress_percent};
    use crate::sftp::{TransferMode, TransferStatus, TransferTask};
    use std::path::PathBuf;
    use transom_core::TransferDirection;

    fn task(
        direction: TransferDirection,
        status: TransferStatus,
        size: Option<u64>,
        transferred: u64,
    ) -> TransferTask {
        let mut task = TransferTask::new(
            direction,
            PathBuf::from("/tmp/app.log"),
            "/srv/app.log".to_string(),
            size,
            TransferMode::Overwrite,
        );
        task.status = status;
        task.transferred = transferred;
        task
    }

    #[test]
    fn progress_percent_handles_unknown_and_overflow() {
        assert_eq!(
            progress_percent(&task(
                TransferDirection::Upload,
                TransferStatus::Transferring,
                None,
                10
            )),
            None
        );
        assert_eq!(
            progress_percent(&task(
                TransferDirection::Upload,
                TransferStatus::Transferring,
                Some(0),
                10
            )),
            None
        );
        let half = progress_percent(&task(
            TransferDirection::Upload,
            TransferStatus::Transferring,
            Some(200),
            100,
        ));
        assert_eq!(half, Some(50.0));
        let overflow = progress_percent(&task(
            TransferDirection::Upload,
            TransferStatus::Transferring,
            Some(100),
            150,
        ));
        assert_eq!(overflow, Some(100.0), "超出总大小的进度必须钳到 100%");
    }

    #[test]
    fn display_name_prefers_local_file_name() {
        let upload = task(
            TransferDirection::Upload,
            TransferStatus::Pending,
            Some(1),
            0,
        );
        assert_eq!(display_name(&upload), "app.log");
    }

    #[test]
    fn display_name_shows_remote_copy_direction() {
        let mut copy = task(
            TransferDirection::RemoteCopy,
            TransferStatus::Pending,
            Some(1),
            0,
        );
        copy.remote_path = "/data/report.csv".to_string();
        copy.target_remote_path = Some("/backup/report.csv".to_string());
        assert_eq!(display_name(&copy), "report.csv");

        copy.target_remote_path = Some("/backup/report-2026.csv".to_string());
        assert_eq!(display_name(&copy), "report.csv → report-2026.csv");
    }

    #[test]
    fn busy_covers_pending_transferring_and_paused() {
        for status in [
            TransferStatus::Pending,
            TransferStatus::Transferring,
            TransferStatus::Paused,
        ] {
            assert!(is_busy(&task(
                TransferDirection::Upload,
                status.clone(),
                Some(1),
                0
            )));
        }
        for status in [
            TransferStatus::Completed,
            TransferStatus::Cancelled,
            TransferStatus::Failed("失败".to_string()),
        ] {
            assert!(!is_busy(&task(
                TransferDirection::Upload,
                status,
                Some(1),
                0
            )));
        }
    }
}
