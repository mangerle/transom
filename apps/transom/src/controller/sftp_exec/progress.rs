//! 传输进度节流回调与平滑速率估算。

use transom_core::{AppEvent, ConnectionId, TabId, TransferId};

/// 传输进度事件的最小发送间隔。
///
/// 传输层每 64KB 回调一次，内网下每秒可达数千次；直接透传会让 UI 以相同
/// 频率整帧重绘。进度是单调可覆盖的状态量且终值由 `SftpFinished` 事件兜底，
/// 中间样本按 100ms 窗口抽样丢弃不损失最终一致性，人眼对进度条刷新的
/// 分辨率也远低于 10Hz。
const PROGRESS_EVENT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// 构造按 [`PROGRESS_EVENT_INTERVAL`] 节流且计算平滑速率与预估时间的进度回调。
pub(crate) fn throttled_progress(
    progress_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
) -> impl FnMut(u64, Option<u64>) + Send {
    let mut last_sent: Option<std::time::Instant> = None;
    let mut last_transferred: u64 = 0;
    let mut speed_ema: Option<f64> = None;
    move |transferred, total| {
        let now = std::time::Instant::now();
        if last_sent.is_none_or(|at| now.duration_since(at) >= PROGRESS_EVENT_INTERVAL) {
            let (speed_bps, eta_secs) = if let Some(last_at) = last_sent {
                let elapsed = now.duration_since(last_at).as_secs_f64();
                if elapsed > 0.0 {
                    let delta_bytes = transferred.saturating_sub(last_transferred);
                    let instant = delta_bytes as f64 / elapsed;
                    let smoothed = match speed_ema {
                        Some(prev) => prev * 0.7 + instant * 0.3,
                        None => instant,
                    };
                    speed_ema = Some(smoothed);
                    let smoothed_bps = smoothed as u64;
                    let eta = if smoothed_bps > 0 {
                        total.map(|tot| tot.saturating_sub(transferred) / smoothed_bps)
                    } else {
                        None
                    };
                    (Some(smoothed_bps), eta)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
            last_sent = Some(now);
            last_transferred = transferred;
            // 回调运行在非 async 上下文：队列满时快速失败丢弃本条样本。
            // 这是显式背压——UI 消费停滞时宁可进度刷新变慢，也不阻塞传输线程。
            let _ = progress_tx.try_send(AppEvent::SftpProgress {
                tab_id,
                connection_id,
                op_id,
                transferred,
                total,
                speed_bps,
                eta_secs,
            });
        }
    }
}
