//! 设置中心"软件更新"分组：检查、下载、验签、安装与重启的完整闭环。
//!
//! # 设计原理（Why）
//! - 自更新是低频重操作，网络请求与磁盘替换全部交给 [`spawn_blocking_local`]
//!   的专用线程，不占用 GPUI 后台执行器，也不在 UI 线程做同步 IO。
//! - 状态机只表达互斥阶段（检查中/可下载/下载中/待安装/安装中/已安装/失败），
//!   避免用多个布尔值拼出非法组合；渲染只读状态，动作入口在用户点击时触发。
//! - 下载进度通过有界单槽通道回传：UI 消费不过来时丢弃旧进度而不是无限积压，
//!   最终结果走同一条通道的 `Finished` 消息，保证"最后一条"不会被进度覆盖。
//!
//! # 代价与局限
//! 更新包暂存目录由 shipup 管理，用户取消或关闭应用后可能残留一个临时文件；
//! 下次安装会复用或覆盖，不影响正确性。安装完成后必须重启才能加载新二进制，
//! 本模块只负责置位重启意图，真正的进程拉起交给 `main` 的优雅停机路径。

use crate::app::spawn_blocking_local;
use crate::palette::Palette;
use crate::settings::SettingsView;
use crate::theme::tokens::{space, typography};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_updater::{DownloadedUpdate, UpdateEvent, UpdateInfo, UpdateStage};

/// 关于页更新区的运行期状态。
#[derive(Default)]
pub(crate) enum UpdateUiState {
    /// 尚未检查。
    #[default]
    Idle,
    /// 正在读取远端清单。
    Checking,
    /// 当前已是最新版本。
    UpToDate,
    /// 发现可下载的新版本。
    Available { info: UpdateInfo },
    /// 正在下载并验签。
    Downloading {
        version: String,
        progress: f32,
        speed_bytes_per_sec: Option<u64>,
        stage: Option<UpdateStage>,
        retry: Option<(u32, u32)>,
    },
    /// 已下载并验签，等待安装。
    Ready { downloaded: DownloadedUpdate },
    /// 正在执行安装。
    Installing { version: String },
    /// 安装完成，等待重启。
    Installed { version: String },
    /// 检查、下载或安装失败。
    Error { message: String },
}

/// 关于页更新区的实体私有状态。
pub(crate) struct UpdatePanel {
    state: UpdateUiState,
    /// 在途任务句柄：随设置实体生死，重新发起时替换旧任务。
    tasks: Vec<Task<()>>,
}

impl Default for UpdatePanel {
    fn default() -> Self {
        Self {
            state: UpdateUiState::Idle,
            tasks: Vec::new(),
        }
    }
}

/// 下载线程回传给 UI 的消息。
pub(super) enum DownloadMessage {
    /// 下载进度或阶段变化。
    Progress(UpdateEvent),
    /// 下载结束；`Err` 为可直接展示的中文原因。
    Finished(Box<Result<DownloadedUpdate, String>>),
}

/// 渲染所需的只读快照：先把状态投影成拥有型数据，避免渲染期间长期借用状态。
pub(super) struct UpdateSnapshot {
    status: String,
    detail: Option<String>,
    notes: Option<String>,
    progress: Option<f32>,
    indeterminate: bool,
    can_check: bool,
    can_download: bool,
    can_install: bool,
    can_restart: bool,
    failed: bool,
}

impl UpdatePanel {
    /// 状态是否处于不可重复触发的在途阶段。
    fn is_busy(&self) -> bool {
        matches!(
            self.state,
            UpdateUiState::Checking
                | UpdateUiState::Downloading { .. }
                | UpdateUiState::Installing { .. }
        )
    }

    /// 把状态投影为渲染快照。
    fn snapshot(&self) -> UpdateSnapshot {
        match &self.state {
            UpdateUiState::Idle => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_idle").to_string(),
                detail: None,
                notes: None,
                progress: None,
                indeterminate: false,
                can_check: true,
                can_download: false,
                can_install: false,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::Checking => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_checking").to_string(),
                detail: None,
                notes: None,
                progress: None,
                indeterminate: true,
                can_check: false,
                can_download: false,
                can_install: false,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::UpToDate => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_up_to_date").to_string(),
                detail: None,
                notes: None,
                progress: None,
                indeterminate: false,
                can_check: true,
                can_download: false,
                can_install: false,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::Available { info } => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_available", version = info.version())
                    .to_string(),
                detail: info.pub_date(),
                notes: info.notes(),
                progress: None,
                indeterminate: false,
                can_check: false,
                can_download: true,
                can_install: false,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::Downloading {
                version,
                progress,
                speed_bytes_per_sec,
                stage,
                retry,
            } => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_downloading", version = version).to_string(),
                detail: Some(download_detail(
                    *progress,
                    *speed_bytes_per_sec,
                    *stage,
                    *retry,
                )),
                notes: None,
                progress: Some(*progress),
                indeterminate: false,
                can_check: false,
                can_download: false,
                can_install: false,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::Ready { downloaded } => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_ready", version = downloaded.version())
                    .to_string(),
                detail: None,
                notes: None,
                progress: Some(100.0),
                indeterminate: false,
                can_check: false,
                can_download: false,
                can_install: true,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::Installing { version } => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_installing", version = version).to_string(),
                detail: None,
                notes: None,
                progress: None,
                indeterminate: true,
                can_check: false,
                can_download: false,
                can_install: false,
                can_restart: false,
                failed: false,
            },
            UpdateUiState::Installed { version } => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_installed", version = version).to_string(),
                detail: Some(rust_i18n::t!("settings.update_restart_hint").to_string()),
                notes: None,
                progress: Some(100.0),
                indeterminate: false,
                can_check: false,
                can_download: false,
                can_install: false,
                can_restart: true,
                failed: false,
            },
            UpdateUiState::Error { message } => UpdateSnapshot {
                status: rust_i18n::t!("settings.update_failed", message = message).to_string(),
                detail: None,
                notes: None,
                progress: None,
                indeterminate: false,
                can_check: true,
                can_download: false,
                can_install: false,
                can_restart: false,
                failed: true,
            },
        }
    }
}

/// 下载阶段的中文说明：进度、速率与阶段合并成一行，避免界面出现空白等待。
pub(super) fn download_detail(
    progress: f32,
    speed_bytes_per_sec: Option<u64>,
    stage: Option<UpdateStage>,
    retry: Option<(u32, u32)>,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(3);
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
fn format_speed(bytes_per_sec: u64) -> String {
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


mod view;
#[cfg(test)]
mod tests;
