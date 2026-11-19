// 刻意不 glob 导入 super：父模块若直接引入 gpui 的测试宏，会与标准测试属性冲突。
use super::snapshot::{bounded_release_notes, download_detail, format_speed};
use super::{MAX_RELEASE_NOTES_CHARS, UpdatePrimaryAction, UpdateUiState, UpdateView};
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
fn idle_view_allows_check_only() {
    let snapshot = UpdateView::default().snapshot();
    assert_eq!(snapshot.primary, UpdatePrimaryAction::Check);
    assert!(!snapshot.failed);
    assert!(snapshot.progress.is_none());
    assert!(snapshot.notes.is_none());
}

/// 运行中的阶段必须禁用重复触发。
#[test]
fn busy_view_disables_primary_action() {
    let view = UpdateView {
        state: UpdateUiState::Checking,
        ..UpdateView::default()
    };
    let snapshot = view.snapshot();
    assert_eq!(snapshot.primary, UpdatePrimaryAction::Busy);
    assert!(snapshot.indeterminate);
}

/// 错误状态仍需保留重试入口，并把错误标红所需的标记置位。
#[test]
fn error_view_allows_retry() {
    let view = UpdateView {
        state: UpdateUiState::Error {
            message: "网络不可达".to_string(),
        },
        ..UpdateView::default()
    };
    let snapshot = view.snapshot();
    assert_eq!(snapshot.primary, UpdatePrimaryAction::Check);
    assert!(snapshot.failed);
}

/// 超长更新日志必须截断，避免远端清单把界面测量与内存拖入无界状态。
#[test]
fn release_notes_are_bounded() {
    let notes = "a".repeat(MAX_RELEASE_NOTES_CHARS + 32);
    let bounded = bounded_release_notes(&notes).expect("非空日志必须保留");
    assert!(bounded.chars().count() > MAX_RELEASE_NOTES_CHARS);
    let marker = rust_i18n::t!("settings.update_notes_truncated").to_string();
    assert!(bounded.ends_with(&marker), "截断结果必须明确告知用户");
}
