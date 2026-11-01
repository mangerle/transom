//! 云同步拉取后的合并、安全副本与缺失处理。

use super::common::*;
use crate::app::app_settings;
use crate::app::update_app_settings;

impl AppShell {
    pub(crate) fn finish_cloud_sync_pull(
        &mut self,
        document: BackupDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let safety_path = self.write_sync_safety_copy();
        if let Some(path) = &safety_path {
            log::info!("下载合并前已保存本机安全副本，路径: {}", path.display());
        }

        let backup = self.state.services.backup.clone();
        let (catalog, secrets) = document.into_parts();
        let task = cx.spawn_in(window, async move |this, cx| {
            let secrets = cx
                .background_executor()
                .spawn(async move {
                    match secrets {
                        Some(secrets) => backup.restore_secrets(&secrets),
                        None => Ok(SecretRestoreReport::default()),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_cloud_sync_pull(catalog, secrets, safety_path, cx);
            });
        });
        self.state.tasks.sync_pull = Some(task);
    }

    /// 把合并前的本机目录导出为明文安全副本；失败不阻断合并，只记日志。
    fn write_sync_safety_copy(&self) -> Option<PathBuf> {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path =
            transom_platform::paths::config_dir().join(format!("sync-safety-{timestamp}.json"));
        let options = BackupExportOptions::default();
        match self
            .state
            .services
            .backup
            .export(&path, &self.state.catalog, &options)
        {
            Ok(_) => Some(path),
            Err(error) => {
                log::warn!(
                    "云同步安全副本写入失败，继续执行合并，路径: {}，原因: {}",
                    path.display(),
                    localized_error(&error)
                );
                None
            }
        }
    }

    /// 合并远端快照：目录合并、时间戳与状态栏回报一次完成。
    fn apply_cloud_sync_pull(
        &mut self,
        catalog: SessionCatalog,
        secrets: Result<SecretRestoreReport, AppError>,
        safety_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let (merged, report) =
            merge_catalogs(None, &self.state.catalog, &catalog, MergePreference::Remote);
        self.state.catalog = merged;
        let saved = self.state.persist_catalog();
        self.state.refresh_tab_sessions();
        update_app_settings(cx, |s| {
            s.sync.last_pull_at = Some(chrono::Utc::now().timestamp())
        });
        let settings_snapshot = app_settings(cx).clone();
        if let Err(error) = self.state.persist_settings(&settings_snapshot) {
            self.state.set_error(localized_error(&error));
        }

        let mut message = rust_i18n::t!(
            "sync.pulled",
            added = report.sessions_added,
            updated = report.sessions_updated,
            folders = report.folders_added
        )
        .to_string();
        if report.conflicts > 0 {
            message.push(' ');
            message
                .push_str(rust_i18n::t!("sync.merge_conflicts", count = report.conflicts).as_ref());
        }
        message.push_str(&secret_restore_note(secrets));
        if let Some(path) = safety_path {
            message.push(' ');
            message.push_str(
                rust_i18n::t!("sync.safety_note", path = path.display().to_string())
                    .to_string()
                    .as_str(),
            );
        }
        if saved {
            self.state.status = Some(message);
        }
        self.state.tasks.sync_pull = None;
        cx.notify();
    }

    /// 同步端点上还没有备份：首次拉取属预期状态，按提示而非错误展示。
    pub(crate) fn handle_cloud_sync_missing(&mut self, cx: &mut Context<Self>) {
        self.state.status = Some(rust_i18n::t!("sync.missing_remote").to_string());
        cx.notify();
    }
}
