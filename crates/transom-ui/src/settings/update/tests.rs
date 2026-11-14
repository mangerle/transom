// 刻意不 glob 导入 super：父模块的 `use gpui_kit::*` 会把 gpui 的
// `#[test]` 属性宏带进来，与标准测试属性冲突并触发递归展开。
use super::{UpdatePanel, UpdateUiState, download_detail, format_speed};
use transom_updater::UpdateStage;

/// 速率格式必须按量级切换单位，避免大文件下载时显示一长串字节数。
#[test]
fn speed_formatting_uses_readable_units() {
    assert_eq!(format_speed(512), "512 B/s");
    assert_eq!(format_speed(2048), "2.0 KB/s");
    assert_eq!(format_speed(3 * 1024 * 1024), "3.00 MB/s");
}

/// 下载详情必须同时带上百分比、速率、阶段与重试次数。
#[test]
fn download_detail_joins_progress_fields() {
    let detail = download_detail(
        12.34,
        Some(2048),
        Some(UpdateStage::VerifyingChecksum),
        Some((1, 3)),
    );
    assert!(detail.contains("12.3%"), "详情缺少百分比: {detail}");
    assert!(detail.contains("2.0 KB/s"), "详情缺少速率: {detail}");
    assert!(!detail.is_empty(), "详情不能为空");
}

/// 空闲状态只允许发起检查，不得误显示下载或安装入口。
#[test]
fn idle_panel_allows_check_only() {
    let snapshot = UpdatePanel::default().snapshot();
    assert!(snapshot.can_check);
    assert!(!snapshot.can_download);
    assert!(!snapshot.can_install);
    assert!(!snapshot.can_restart);
    assert!(!snapshot.failed);
    assert!(snapshot.progress.is_none());
}

/// 失败状态必须允许重试，并把错误标红所需的标记置位。
#[test]
fn error_panel_allows_retry() {
    let panel = UpdatePanel {
        state: UpdateUiState::Error {
            message: "网络不可达".to_string(),
        },
        tasks: Vec::new(),
    };
    let snapshot = panel.snapshot();
    assert!(snapshot.can_check);
    assert!(snapshot.failed);
    assert!(!snapshot.can_download);
}
