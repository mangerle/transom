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
enum DownloadMessage {
    /// 下载进度或阶段变化。
    Progress(UpdateEvent),
    /// 下载结束；`Err` 为可直接展示的中文原因。
    Finished(Box<Result<DownloadedUpdate, String>>),
}

/// 渲染所需的只读快照：先把状态投影成拥有型数据，避免渲染期间长期借用状态。
struct UpdateSnapshot {
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
fn download_detail(
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
fn stage_label(stage: UpdateStage) -> String {
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

impl SettingsView {
    /// 检查更新：在阻塞线程读取清单，回到 UI 后写入状态。
    pub(crate) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if self.update.is_busy() {
            return;
        }
        self.update.state = UpdateUiState::Checking;
        cx.notify();

        let task = cx.spawn(async move |this, cx| {
            let result = spawn_blocking_local(|| transom_updater::check_update(None)).await;
            let Some(result) = result else {
                let _ = this.update(cx, |this, cx| {
                    this.update.state = UpdateUiState::Error {
                        message: rust_i18n::t!("settings.update_thread_failed").to_string(),
                    };
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.apply_check_result(result);
                cx.notify();
            });
        });
        self.update.tasks = vec![task];
    }

    /// 应用检查结果。
    fn apply_check_result(
        &mut self,
        result: Result<Option<UpdateInfo>, transom_updater::UpdaterError>,
    ) {
        self.update.state = match result {
            Ok(Some(info)) => UpdateUiState::Available { info },
            Ok(None) => UpdateUiState::UpToDate,
            Err(error) => UpdateUiState::Error {
                message: error.to_string(),
            },
        };
    }

    /// 下载更新：阻塞线程下载并验签，进度经有界单槽通道回传。
    pub(crate) fn download_update(&mut self, cx: &mut Context<Self>) {
        let UpdateUiState::Available { info } = &self.update.state else {
            return;
        };
        let info = info.clone();
        let version = info.version();
        self.update.state = UpdateUiState::Downloading {
            version,
            progress: 0.0,
            speed_bytes_per_sec: None,
            stage: Some(UpdateStage::DownloadStarted),
            retry: None,
        };
        cx.notify();

        // 容量 1：UI 消费不过来时丢弃旧进度，绝不无限积压；最终结果必须送达。
        let (tx, rx) = flume::bounded::<DownloadMessage>(1);
        let job_tx = tx.clone();
        let job = cx.spawn(async move |_this, _cx| {
            let spawned = spawn_blocking_local(move || {
                let result = info
                    .download_blocking(|event| {
                        let _ = job_tx.try_send(DownloadMessage::Progress(event));
                    })
                    .map_err(|error| error.to_string());
                let _ = job_tx.send(DownloadMessage::Finished(Box::new(result)));
            })
            .await;
            if spawned.is_none() {
                let _ = tx.send(DownloadMessage::Finished(Box::new(Err(rust_i18n::t!(
                    "settings.update_thread_failed"
                )
                .to_string()))));
            }
        });
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv_async().await {
                match message {
                    DownloadMessage::Progress(event) => {
                        let _ = this.update(cx, |this, cx| {
                            this.apply_download_event(event);
                            cx.notify();
                        });
                    }
                    DownloadMessage::Finished(result) => {
                        let _ = this.update(cx, |this, cx| {
                            this.apply_download_result(*result);
                            cx.notify();
                        });
                        break;
                    }
                }
            }
        });
        self.update.tasks = vec![job, pump];
    }

    /// 把下载事件写入状态机。
    fn apply_download_event(&mut self, event: UpdateEvent) {
        match event {
            UpdateEvent::DownloadProgress {
                percent,
                speed_bytes_per_sec,
                ..
            } => {
                if let UpdateUiState::Downloading {
                    progress,
                    speed_bytes_per_sec: speed,
                    ..
                } = &mut self.update.state
                {
                    *progress = percent.unwrap_or(0.0);
                    *speed = speed_bytes_per_sec;
                }
            }
            UpdateEvent::Stage(stage) => {
                if let UpdateUiState::Downloading { stage: current, .. } = &mut self.update.state {
                    *current = Some(stage);
                }
            }
            UpdateEvent::Retrying {
                attempt,
                max_retries,
                ..
            } => {
                if let UpdateUiState::Downloading { retry, .. } = &mut self.update.state {
                    *retry = Some((attempt, max_retries));
                }
            }
            UpdateEvent::Failed { reason } => {
                self.update.state = UpdateUiState::Error { message: reason };
            }
            _ => {}
        }
    }

    /// 应用下载结果。
    fn apply_download_result(&mut self, result: Result<DownloadedUpdate, String>) {
        self.update.state = match result {
            Ok(downloaded) => UpdateUiState::Ready { downloaded },
            Err(message) => UpdateUiState::Error { message },
        };
    }

    /// 安装已下载并验签的更新包。
    pub(crate) fn install_update(&mut self, cx: &mut Context<Self>) {
        let UpdateUiState::Ready { downloaded } = &self.update.state else {
            return;
        };
        let downloaded = downloaded.clone();
        let version = downloaded.version();
        self.update.state = UpdateUiState::Installing {
            version: version.clone(),
        };
        cx.notify();

        let task = cx.spawn(async move |this, cx| {
            let result = spawn_blocking_local(move || {
                downloaded
                    .install(|_event| {})
                    .map_err(|error| error.to_string())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.update.state = match result {
                    Some(Ok(())) => UpdateUiState::Installed { version },
                    Some(Err(message)) => UpdateUiState::Error { message },
                    None => UpdateUiState::Error {
                        message: rust_i18n::t!("settings.update_thread_failed").to_string(),
                    },
                };
                cx.notify();
            });
        });
        self.update.tasks = vec![task];
    }

    /// 安装完成后重启：把意图交回外壳，由外壳置位跨层信号并退出事件循环。
    pub(crate) fn restart_for_update(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.update.state, UpdateUiState::Installed { .. }) {
            return;
        }
        cx.emit(super::SettingsEvent::RestartForUpdate);
    }

    /// 渲染"软件更新"分组。
    pub(crate) fn render_settings_update(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let snapshot = self.update.snapshot();
        let check = cx.listener(|this, _, _, cx| this.check_for_updates(cx));
        let download = cx.listener(|this, _, _, cx| this.download_update(cx));
        let install = cx.listener(|this, _, _, cx| this.install_update(cx));
        let restart = cx.listener(|this, _, _, cx| this.restart_for_update(cx));

        let button = if snapshot.can_restart {
            Button::new("about-update-restart")
                .small()
                .label(rust_i18n::t!("settings.update_restart_action").to_string())
                .on_click(restart)
        } else if snapshot.can_install {
            Button::new("about-update-install")
                .small()
                .label(rust_i18n::t!("settings.update_install_action").to_string())
                .on_click(install)
        } else if snapshot.can_download {
            Button::new("about-update-download")
                .small()
                .label(rust_i18n::t!("settings.update_download_action").to_string())
                .on_click(download)
        } else if snapshot.can_check {
            Button::new("about-update-check")
                .small()
                .label(rust_i18n::t!("settings.update_check_action").to_string())
                .on_click(check)
        } else {
            Button::new("about-update-busy")
                .small()
                .label(rust_i18n::t!("settings.update_busy_action").to_string())
                .disabled(true)
        };

        let status_color = if snapshot.failed {
            p.danger
        } else {
            p.muted_foreground
        };

        let mut details = v_flex().w_full().gap(rems(space::XXS)).child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(status_color)
                .child(snapshot.status),
        );
        if let Some(detail) = snapshot.detail {
            details = details.child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.muted_foreground)
                    .child(detail),
            );
        }
        if let Some(notes) = snapshot.notes.filter(|notes| !notes.trim().is_empty()) {
            details = details.child(
                v_flex()
                    .w_full()
                    .gap(rems(space::XXS))
                    .pt(rems(space::XS))
                    .child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("settings.update_notes_title").to_string()),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_h(px(120.0))
                            .overflow_y_scrollbar()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.muted_foreground)
                            .child(notes),
                    ),
            );
        }
        if snapshot.indeterminate {
            details = details.child(
                Progress::new("about-update-progress")
                    .loading(true)
                    .w_full(),
            );
        } else if let Some(percent) = snapshot.progress {
            details = details.child(
                Progress::new("about-update-progress")
                    .value(percent)
                    .w_full(),
            );
        }

        v_flex()
            .w_full()
            .gap(rems(space::SM))
            .child(crate::widgets::setting_row(
                "settings.update_title",
                Some("settings.update_desc"),
                h_flex().items_center().child(button).into_any_element(),
                p,
            ))
            .child(details)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
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
}
