use super::*;
use crate::app::app_settings;

/// 队列与状态栏的渲染数据快照。
pub(super) struct SftpQueueStatusData {
    pub(super) queue_height: f32,
    pub(super) remote_error: Option<String>,
    pub(super) status: Option<String>,
    pub(super) local_error: Option<String>,
    pub(super) queue: Vec<crate::sftp::TransferTask>,
    pub(super) total_speed_bps: Option<u64>,
    pub(super) resume_enabled: bool,
    pub(super) verify_after_transfer: bool,
    pub(super) max_concurrent_transfers: usize,
    pub(super) queue_expanded: bool,
    pub(super) paused_count: usize,
    pub(super) active_count: usize,
    pub(super) display_progress: Option<(u64, Option<u64>)>,
}

impl SftpPanelView {
    /// 队列与状态栏所需数据：设置、任务快照、活动/暂停计数与显示进度。
    pub(super) fn queue_status_data(
        &self,
        state: &super::state::SftpPanelState,
        cx: &mut Context<Self>,
    ) -> SftpQueueStatusData {
        let settings = app_settings(cx).sftp.clone();
        let queue_height = state.queue.height;
        let remote_error = state.queue.error.clone();
        let status = state.queue.status.clone();
        let progress = state.queue.progress;
        let active_count = state.queue.active_transfers.len();

        let local_error = state.local.error.clone();

        let queue = state.queue.tasks.clone();
        let total_speed: u64 = queue
            .iter()
            .filter(|t| t.status == TransferStatus::Transferring)
            .filter_map(|t| t.speed_bps)
            .sum();
        let total_speed_bps = if total_speed > 0 {
            Some(total_speed)
        } else {
            None
        };
        let resume_enabled = state.queue.resume_enabled;
        let verify_after_transfer = settings.verify_after_transfer;
        let max_concurrent_transfers = settings.max_concurrent_transfers.clamp(1, 8);
        let queue_expanded = state.queue.expanded;
        // 没有活动任务时回退汇总暂停任务进度，避免暂停后进度条消失
        let paused_count = state
            .queue
            .tasks
            .iter()
            .filter(|task| task.status == TransferStatus::Paused)
            .count();
        let paused_tasks = if active_count == 0 {
            state
                .queue
                .tasks
                .iter()
                .filter(|task| task.status == TransferStatus::Paused)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let display_progress = if active_count == 0 && !paused_tasks.is_empty() {
            let transferred = paused_tasks
                .iter()
                .fold(0u64, |sum, task| sum.saturating_add(task.transferred));
            let mut total_known = true;
            let total = paused_tasks.iter().fold(0u64, |sum, task| match task.size {
                Some(size) => sum.saturating_add(size),
                None => {
                    total_known = false;
                    sum
                }
            });
            Some((transferred, total_known.then_some(total)))
        } else {
            progress
        };

        SftpQueueStatusData {
            queue_height,
            remote_error,
            status,
            local_error,
            queue,
            total_speed_bps,
            resume_enabled,
            verify_after_transfer,
            max_concurrent_transfers,
            queue_expanded,
            paused_count,
            active_count,
            display_progress,
        }
    }
}
