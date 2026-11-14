//! 软件更新分组的视图渲染与交互动作。

use super::*;

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
        cx.emit(crate::settings::SettingsEvent::RestartForUpdate);
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

