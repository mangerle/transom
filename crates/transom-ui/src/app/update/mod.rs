//! 窗口级应用更新实体：启动检查、标题栏提示、更新弹窗与安装闭环。
//!
//! # 设计原理（Why）
//! - 更新状态的生命周期属于窗口，而不是设置页。设置弹窗关闭后，启动检查结果
//!   仍要驱动标题栏版本徽标；因此状态机、任务句柄与弹窗开关集中在本实体，
//!   设置页只负责发出“检查更新”意图。
//! - 启动检查只读取一次远端清单，发现新版本时仅点亮标题栏版本徽标，不主动
//!   打断用户；从设置页发起检查时则直接打开同一弹窗，保证入口共享唯一状态。
//! - 网络、下载、验签与安装全部交给 [`spawn_blocking_local`]，UI 线程只消费
//!   有界通道消息，避免阻塞渲染。
//!
//! # 代价与局限
//! 下载进度只保留最新值，不保存历史曲线；关闭弹窗不会取消后台下载，但标题栏
//! 徽标会继续提示，用户可随时重新打开查看进度。

use crate::app::spawn_blocking_local;
use gpui_kit::*;
use transom_updater::{DownloadedUpdate, UpdateEvent as UpdaterEvent, UpdateInfo, UpdateStage};

/// 更新日志展示上限：远端清单属于外部输入，超长正文不允许直接铺满内存与界面。
pub(super) const MAX_RELEASE_NOTES_CHARS: usize = 64 * 1024;

/// 更新流程的互斥阶段。
#[derive(Default)]
pub(super) enum UpdateUiState {
    /// 尚未检查。
    #[default]
    Idle,
    /// 正在读取远端清单。
    Checking,
    /// 当前已是最新版本。
    UpToDate,
    /// 发现新版本，等待下载。
    Available,
    /// 正在下载并验签。
    Downloading {
        version: String,
        progress: f32,
        speed_bytes_per_sec: Option<u64>,
        stage: Option<UpdateStage>,
        retry: Option<(u32, u32)>,
    },
    /// 已下载并验签，等待安装。
    Ready,
    /// 正在执行安装。
    Installing { version: String },
    /// 安装完成，等待重启。
    Installed { version: String },
    /// 最近一次检查、下载或安装失败。
    Error { message: String },
}

/// 弹窗主动作，由当前可用数据决定，避免界面把“有问题”误投影成“可检查”。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdatePrimaryAction {
    Check,
    Download,
    Install,
    Restart,
    Busy,
}

/// 更新实体私有状态。
///
/// `available`、`downloaded` 与 `installed` 与阶段分开保存：下载或安装失败时
/// 仍保留前一步成果，重试不需要重新检查清单，也不会丢失更新日志。
pub(crate) struct UpdateView {
    state: UpdateUiState,
    available: Option<UpdateInfo>,
    downloaded: Option<DownloadedUpdate>,
    installed: Option<String>,
    open: bool,
    tasks: Vec<Task<()>>,
    startup_check_started: bool,
}

impl Default for UpdateView {
    fn default() -> Self {
        Self {
            state: UpdateUiState::Idle,
            available: None,
            downloaded: None,
            installed: None,
            open: false,
            tasks: Vec::new(),
            startup_check_started: false,
        }
    }
}

/// 更新实体向外壳发出的窗口级意图。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateEvent {
    /// 更新已安装，需要优雅停机后重启，使新二进制生效。
    RestartRequested,
}

impl EventEmitter<UpdateEvent> for UpdateView {}

impl UpdateView {
    /// 创建空闲更新实体。
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 应用启动后执行一次检查；重复调用不会再次请求远端。
    pub(crate) fn start_startup_check(&mut self, cx: &mut Context<Self>) {
        if self.startup_check_started {
            return;
        }
        self.startup_check_started = true;
        self.begin_check(cx, false);
    }

    /// 用户从标题栏或设置页打开更新弹窗；没有结果时立即发起检查。
    pub(crate) fn open_and_check(&mut self, cx: &mut Context<Self>) {
        self.open = true;
        if self.available.is_none() && self.downloaded.is_none() && !self.is_busy() {
            self.begin_check(cx, false);
        } else {
            cx.notify();
        }
    }

    /// 关闭弹窗；后台任务继续运行，标题栏提示仍表示可继续查看的更新状态。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
    }

    /// 弹窗当前是否应当出现在窗口弹窗栈中。
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// 是否需要在标题栏版本徽标上显示更新提示。
    pub(crate) fn has_update(&self) -> bool {
        self.available.is_some() || self.installed.is_some()
    }

    /// 当前可展示的新版本号；没有已知更新时返回 `None`。
    pub(crate) fn available_version(&self) -> Option<String> {
        self.available.as_ref().map(UpdateInfo::version)
    }

    /// 当前阶段是否处于不可重复触发的在途状态。
    fn is_busy(&self) -> bool {
        matches!(
            self.state,
            UpdateUiState::Checking
                | UpdateUiState::Downloading { .. }
                | UpdateUiState::Installing { .. }
        )
    }

    /// 发起一次清单检查；`prompt_on_available` 为真时结果会直接打开弹窗。
    fn begin_check(&mut self, cx: &mut Context<Self>, prompt_on_available: bool) {
        if self.is_busy() {
            return;
        }
        self.state = UpdateUiState::Checking;
        self.available = None;
        self.downloaded = None;
        self.installed = None;
        cx.notify();

        let task = cx.spawn(async move |this, cx| {
            let result = spawn_blocking_local(|| transom_updater::check_update(None)).await;
            let Some(result) = result else {
                let _ = this.update(cx, |this, cx| {
                    this.state = UpdateUiState::Error {
                        message: rust_i18n::t!("settings.update_thread_failed").to_string(),
                    };
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.apply_check_result(result, prompt_on_available);
                cx.notify();
            });
        });
        self.tasks = vec![task];
    }

    /// 应用检查结果；发现更新时保存完整信息供后续下载与弹窗复用。
    fn apply_check_result(
        &mut self,
        result: Result<Option<UpdateInfo>, transom_updater::UpdaterError>,
        prompt_on_available: bool,
    ) {
        self.state = match result {
            Ok(Some(info)) => {
                self.available = Some(info);
                if prompt_on_available {
                    self.open = true;
                }
                UpdateUiState::Available
            }
            Ok(None) => {
                self.available = None;
                UpdateUiState::UpToDate
            }
            Err(error) => UpdateUiState::Error {
                message: error.to_string(),
            },
        };
    }

    /// 下载更新：阻塞线程下载并验签，进度经有界单槽通道回传。
    pub(crate) fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(info) = self.available.clone() else {
            return;
        };
        let version = info.version();
        self.state = UpdateUiState::Downloading {
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
        self.tasks = vec![job, pump];
    }

    /// 把下载事件写入下载阶段。
    fn apply_download_event(&mut self, event: UpdaterEvent) {
        match event {
            UpdaterEvent::DownloadProgress {
                percent,
                speed_bytes_per_sec,
                ..
            } => {
                if let UpdateUiState::Downloading {
                    progress,
                    speed_bytes_per_sec: speed,
                    ..
                } = &mut self.state
                {
                    *progress = percent.unwrap_or(0.0);
                    *speed = speed_bytes_per_sec;
                }
            }
            UpdaterEvent::Stage(stage) => {
                if let UpdateUiState::Downloading { stage: current, .. } = &mut self.state {
                    *current = Some(stage);
                }
            }
            UpdaterEvent::Retrying {
                attempt,
                max_retries,
                ..
            } => {
                if let UpdateUiState::Downloading { retry, .. } = &mut self.state {
                    *retry = Some((attempt, max_retries));
                }
            }
            UpdaterEvent::Failed { reason } => {
                self.state = UpdateUiState::Error { message: reason };
            }
            _ => {}
        }
    }

    /// 应用下载结果；失败时保留已检查到的更新信息，允许直接重试下载。
    fn apply_download_result(&mut self, result: Result<DownloadedUpdate, String>) {
        self.state = match result {
            Ok(downloaded) => {
                self.downloaded = Some(downloaded);
                UpdateUiState::Ready
            }
            Err(message) => UpdateUiState::Error { message },
        };
    }

    /// 安装已下载并验签的更新包。
    pub(crate) fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(downloaded) = self.downloaded.clone() else {
            return;
        };
        let version = downloaded.version();
        self.state = UpdateUiState::Installing {
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
                this.installed = matches!(result.as_ref(), Some(Ok(()))).then(|| version.clone());
                this.state = match result {
                    Some(Ok(())) => UpdateUiState::Installed { version },
                    Some(Err(message)) => UpdateUiState::Error { message },
                    None => UpdateUiState::Error {
                        message: rust_i18n::t!("settings.update_thread_failed").to_string(),
                    },
                };
                cx.notify();
            });
        });
        self.tasks = vec![task];
    }

    /// 执行弹窗主按钮对应的动作。
    pub(crate) fn perform_primary_action_for_dialog(&mut self, cx: &mut Context<Self>) {
        match self.primary_action() {
            UpdatePrimaryAction::Check => self.begin_check(cx, false),
            UpdatePrimaryAction::Download => self.download_update(cx),
            UpdatePrimaryAction::Install => self.install_update(cx),
            UpdatePrimaryAction::Restart => self.restart_for_update(cx),
            UpdatePrimaryAction::Busy => {}
        }
    }

    /// 安装完成后请求外壳退出事件循环；真正的进程替换由 `main` 负责。
    fn restart_for_update(&mut self, cx: &mut Context<Self>) {
        if self.installed.is_some() {
            cx.emit(UpdateEvent::RestartRequested);
        }
    }

    /// 当前主按钮动作。
    fn primary_action(&self) -> UpdatePrimaryAction {
        if self.is_busy() {
            return UpdatePrimaryAction::Busy;
        }
        if self.installed.is_some() {
            UpdatePrimaryAction::Restart
        } else if self.downloaded.is_some() {
            UpdatePrimaryAction::Install
        } else if self.available.is_some() {
            UpdatePrimaryAction::Download
        } else {
            UpdatePrimaryAction::Check
        }
    }
}

/// 下载线程回传给 UI 的消息。
enum DownloadMessage {
    /// 下载进度或阶段变化。
    Progress(UpdaterEvent),
    /// 下载结束；`Err` 为可直接展示的中文原因。
    Finished(Box<Result<DownloadedUpdate, String>>),
}

mod snapshot;
#[cfg(test)]
mod tests;
mod view;
