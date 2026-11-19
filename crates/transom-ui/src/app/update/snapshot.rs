//! 更新状态投影、Markdown 日志边界与阶段文案。
//!
//! 单独拆出投影逻辑，避免核心动作文件同时承担状态机、文案映射与外部输入
//! 限长；渲染只消费本模块产出的拥有型快照。

use super::{MAX_RELEASE_NOTES_CHARS, UpdatePrimaryAction, UpdateUiState, UpdateView};
use transom_updater::{UpdateInfo, UpdateStage};

/// 弹窗渲染所需的只读快照。
pub(super) struct UpdateSnapshot {
    pub(super) status: String,
    pub(super) detail: Option<String>,
    pub(super) notes: Option<String>,
    pub(super) current_version: String,
    pub(super) target_version: Option<String>,
    pub(super) published: Option<String>,
    pub(super) progress: Option<f32>,
    pub(super) indeterminate: bool,
    pub(super) primary: UpdatePrimaryAction,
    pub(super) failed: bool,
}

impl UpdateView {
    /// 把状态投影为拥有型快照，避免渲染期间长期借用状态机。
    pub(super) fn snapshot(&self) -> UpdateSnapshot {
        let target_version = self.available.as_ref().map(UpdateInfo::version);
        let notes = self
            .available
            .as_ref()
            .and_then(UpdateInfo::notes)
            .and_then(|notes| bounded_release_notes(&notes));
        let published = self.available.as_ref().and_then(UpdateInfo::pub_date);
        let failed = matches!(self.state, UpdateUiState::Error { .. });

        let (status, detail, progress, indeterminate) = match &self.state {
            UpdateUiState::Idle => (
                rust_i18n::t!("settings.update_idle").to_string(),
                None,
                None,
                false,
            ),
            UpdateUiState::Checking => (
                rust_i18n::t!("settings.update_checking").to_string(),
                None,
                None,
                true,
            ),
            UpdateUiState::UpToDate => (
                rust_i18n::t!("settings.update_up_to_date").to_string(),
                None,
                None,
                false,
            ),
            UpdateUiState::Available => (
                target_version
                    .as_ref()
                    .map(|version| {
                        rust_i18n::t!("settings.update_available", version = version).to_string()
                    })
                    .unwrap_or_else(|| rust_i18n::t!("settings.update_title").to_string()),
                None,
                None,
                false,
            ),
            UpdateUiState::Downloading {
                version,
                progress,
                speed_bytes_per_sec,
                stage,
                retry,
            } => (
                rust_i18n::t!("settings.update_downloading", version = version).to_string(),
                Some(download_detail(
                    *progress,
                    *speed_bytes_per_sec,
                    *stage,
                    *retry,
                )),
                Some(*progress),
                false,
            ),
            UpdateUiState::Ready => (
                self.downloaded
                    .as_ref()
                    .map(|downloaded| {
                        rust_i18n::t!("settings.update_ready", version = downloaded.version())
                            .to_string()
                    })
                    .unwrap_or_else(|| rust_i18n::t!("settings.update_title").to_string()),
                None,
                Some(100.0),
                false,
            ),
            UpdateUiState::Installing { version } => (
                rust_i18n::t!("settings.update_installing", version = version).to_string(),
                None,
                None,
                true,
            ),
            UpdateUiState::Installed { version } => (
                rust_i18n::t!("settings.update_installed", version = version).to_string(),
                Some(rust_i18n::t!("settings.update_restart_hint").to_string()),
                Some(100.0),
                false,
            ),
            UpdateUiState::Error { message } => (
                rust_i18n::t!("settings.update_failed", message = message).to_string(),
                None,
                None,
                false,
            ),
        };

        UpdateSnapshot {
            status,
            detail,
            notes,
            current_version: crate::APP_VERSION.to_string(),
            target_version,
            published,
            progress,
            indeterminate,
            primary: self.primary_action(),
            failed,
        }
    }
}
/// 截断外部更新日志，既保留 Markdown 结构又限制内存与测量成本。
pub(super) fn bounded_release_notes(notes: &str) -> Option<String> {
    let trimmed = notes.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_RELEASE_NOTES_CHARS {
        return Some(trimmed.to_string());
    }
    let mut bounded: String = trimmed.chars().take(MAX_RELEASE_NOTES_CHARS).collect();
    bounded.push_str(&rust_i18n::t!("settings.update_notes_truncated"));
    Some(bounded)
}

/// 下载阶段的中文说明：进度、速率与阶段合并成一行，避免界面出现空白等待。
pub(super) fn download_detail(
    progress: f32,
    speed_bytes_per_sec: Option<u64>,
    stage: Option<UpdateStage>,
    retry: Option<(u32, u32)>,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(4);
    parts.push(format!("{progress:.1}%"));
    if let Some(speed) = speed_bytes_per_sec {
        parts.push(format_speed(speed));
    }
    if let Some(stage) = stage {
        parts.push(stage_label(stage));
    }
    if let Some((attempt, max_retries)) = retry {
        parts.push(
            rust_i18n::t!(
                "settings.update_retrying",
                attempt = attempt,
                max_retries = max_retries
            )
            .to_string(),
        );
    }
    parts.join(" · ")
}

/// 把字节速率格式化为易读单位。
pub(super) fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1024 * 1024 {
        format!("{:.2} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{bytes_per_sec} B/s")
    }
}

/// 更新阶段的中文文案键。
pub(super) fn stage_label(stage: UpdateStage) -> String {
    let key = match stage {
        UpdateStage::CheckingManifest => "settings.update_stage_checking_manifest",
        UpdateStage::DownloadStarted => "settings.update_stage_download_started",
        UpdateStage::VerifyingChecksum => "settings.update_stage_verifying_checksum",
        UpdateStage::VerifyingSignature => "settings.update_stage_verifying_signature",
        UpdateStage::ExtractingArchive => "settings.update_stage_extracting",
        UpdateStage::Installing => "settings.update_stage_installing",
        UpdateStage::ReadyToRestart => "settings.update_stage_ready_to_restart",
        UpdateStage::DeferredToReboot => "settings.update_stage_deferred_to_reboot",
        UpdateStage::Completed => "settings.update_stage_completed",
        UpdateStage::Other => "settings.update_stage_other",
        _ => "settings.update_stage_other",
    };
    rust_i18n::t!(key).to_string()
}
