//! 云同步拉取后的合并、安全副本与本地回填。

use super::common::*;
use crate::app::app_settings;
use crate::app::update_app_settings;
use transom_core::{CloudPullMode, SyncPullReport, sync_catalog_hash};

/// 下载完成后交给界面线程的合并参数。
struct CloudPullApply {
    remote_catalog: SessionCatalog,
    secrets: Result<SecretRestoreReport, AppError>,
    safety_path: Option<PathBuf>,
    mode: CloudPullMode,
    remote_hash: String,
    at: i64,
}

impl AppShell {
    /// 处理下载结果：常规模式合并，冲突裁决模式以远端目录整体替换。
    pub(crate) fn finish_cloud_sync_pull(
        &mut self,
        report: SyncPullReport,
        mode: CloudPullMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let safety_path = self.write_sync_safety_copy();
        if let Some(path) = &safety_path {
            log::info!("下载合并前已保存本机安全副本，路径: {}", path.display());
        }

        let backup = self.state.services.backup.clone();
        let SyncPullReport {
            mut document,
            remote_version: _,
            remote_hash,
            bytes: _,
            at,
        } = report;
        document.normalize();
        let (remote_catalog, secrets) = document.into_parts();
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
                this.apply_cloud_sync_pull(
                    CloudPullApply {
                        remote_catalog,
                        secrets,
                        safety_path,
                        mode,
                        remote_hash,
                        at,
                    },
                    cx,
                );
            });
        });
        self.state.tasks.background.sync_pull = Some(task);
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

    /// 合并或替换远端快照，并回填同步基线与状态栏提示。
    fn apply_cloud_sync_pull(&mut self, input: CloudPullApply, cx: &mut Context<Self>) {
        let CloudPullApply {
            remote_catalog,
            secrets,
            safety_path,
            mode,
            remote_hash,
            at,
        } = input;
        let (catalog, report) = match mode {
            CloudPullMode::Merge => merge_catalogs(
                None,
                &self.state.catalog,
                &remote_catalog,
                MergePreference::Remote,
            ),
            CloudPullMode::ForceRemote => {
                (remote_catalog, transom_core::SyncMergeReport::default())
            }
        };
        self.state.catalog = catalog;
        let saved = self.state.persist_catalog();
        self.state
            .cloud_sync
            .scheduler
            .mark_catalog_synced(self.state.catalog_revision);
        self.state.refresh_tab_sessions();
        let local_hash = sync_catalog_hash(&self.state.catalog).ok();
        update_app_settings(cx, |s| {
            if let Some(baseline) = s.sync.active_baseline_mut() {
                baseline.last_pull_at = Some(at);
                baseline.last_checked_at = Some(at);
                baseline.last_remote_hash = Some(remote_hash);
                if let Some(local_hash) = local_hash {
                    baseline.last_sync_catalog_hash = Some(local_hash);
                }
            }
        });
        let settings_snapshot = app_settings(cx).clone();
        if let Err(error) = self.state.persist_settings(&settings_snapshot) {
            self.state.set_error(localized_error(&error));
        }

        let mut message = match mode {
            CloudPullMode::Merge => rust_i18n::t!(
                "sync.pulled",
                added = report.sessions_added,
                updated = report.sessions_updated,
                folders = report.folders_added
            )
            .to_string(),
            CloudPullMode::ForceRemote => rust_i18n::t!("sync.pull_force_done").to_string(),
        };
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
            self.state.status = Some(message.clone());
        }
        self.state.cloud_sync.active_operation = None;
        self.state.cloud_sync.active_trigger = None;
        self.state.cloud_sync.status = super::history::SyncHistoryStatus::Success;
        self.state.cloud_sync.message = Some(message);
        self.state.tasks.background.sync_pull = None;
        cx.notify();
    }
}
