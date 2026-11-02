//! 队列状态栏文案单元测试。

use super::status_bar::{SftpStatusBarProps, progress_display};

fn props(progress: Option<(u64, Option<u64>)>, active: usize, paused: usize) -> SftpStatusBarProps {
    SftpStatusBarProps {
        tab_id: transom_core::TabId::new(),
        progress,
        active_count: active,
        paused_count: paused,
        queue_expanded: false,
        remote_error: None,
        local_error: None,
        status: None,
        total_speed_bps: None,
    }
}

/// 已知总量时给出百分比与“已传/总量”，活动任务数拼在文案里。
#[test]
fn progress_display_reports_percent_and_active_count() {
    let (percent, label) = progress_display(&props(Some((50, Some(100))), 1, 0));
    assert!((percent - 50.0).abs() < f32::EPSILON);
    assert!(label.contains("50"));
}

/// 无进度时百分比为 0、文案为空；暂停任务只在无活动任务时提示。
#[test]
fn progress_display_handles_empty_and_paused() {
    let (percent, label) = progress_display(&props(None, 0, 0));
    assert_eq!(percent, 0.0);
    assert!(label.is_empty());
    let (_, paused) = progress_display(&props(Some((10, None)), 0, 1));
    assert!(!paused.is_empty());
}
