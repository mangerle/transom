//! 云端同步的界面编排：设置表单、凭据落钥匙串、上传下载与合并回填。
//!
//! 分层说明（Why）：
//! - 端点拓扑（地址、账号、目录）进设置文件，WebDAV 密码、S3 Secret Key
//!   与同步口令只进系统钥匙串——设置文件可能被导出共享，秘密绝不同行；
//! - 上传下载是分钟级网络操作，走 `AppCommand` 交给控制器的 tokio 运行时，
//!   结果经 `AppEvent` 回流；合并动作回到界面线程执行，因为目录快照是
//!   界面侧的独占状态；
//! - 冲突策略是"后写者胜"，因此下载合并前先把本机目录导出为明文安全
//!   副本（0600），误合并可从文件恢复。

use crate::app::AppShell;
use crate::backup::secret_restore_note;
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    AppCommand, AppError, BackupDocument, BackupExportOptions, GitSyncResult, GitSyncSettings,
    SecretKey, SecretRestoreReport, SessionCatalog, SyncEndpoint, SyncPushReport, SyncSettings,
};

/// 设置弹窗里云同步组的全部输入框。
///
/// 密码、Secret Key 与同步口令为掩码输入且打开弹窗时不回填：钥匙串里
/// 已有的值不回显，只有用户重新输入才会覆盖，避免旁人看到明文。
pub(crate) struct SyncFormInputs {
    pub(crate) webdav_url: Entity<InputState>,
    pub(crate) webdav_username: Entity<InputState>,
    pub(crate) webdav_password: Entity<InputState>,
    pub(crate) s3_endpoint: Entity<InputState>,
    pub(crate) s3_region: Entity<InputState>,
    pub(crate) s3_bucket: Entity<InputState>,
    pub(crate) s3_access_key: Entity<InputState>,
    pub(crate) s3_secret: Entity<InputState>,
    pub(crate) directory: Entity<InputState>,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) git_remote: Entity<InputState>,
    pub(crate) git_branch: Entity<InputState>,
    pub(crate) git_interval: Entity<InputState>,
    pub(crate) git_token: Entity<InputState>,
}

/// 备份对象在端点上的默认子目录。
pub(crate) const DEFAULT_SYNC_DIRECTORY: &str = "transom";

impl SyncFormInputs {
    pub(crate) fn new(window: &mut Window, cx: &mut gpui::App) -> Self {
        fn field(window: &mut Window, cx: &mut gpui::App, placeholder: &str) -> Entity<InputState> {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.to_string()))
        }
        fn secret_field(
            window: &mut Window,
            cx: &mut gpui::App,
            placeholder: &str,
        ) -> Entity<InputState> {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(placeholder.to_string())
            })
        }
        Self {
            webdav_url: field(window, cx, rust_i18n::t!("sync.webdav_url_hint").as_ref()),
            webdav_username: field(window, cx, rust_i18n::t!("sync.username_hint").as_ref()),
            webdav_password: secret_field(window, cx, rust_i18n::t!("sync.password_hint").as_ref()),
            s3_endpoint: field(window, cx, rust_i18n::t!("sync.s3_endpoint_hint").as_ref()),
            s3_region: field(window, cx, "us-east-1"),
            s3_bucket: field(window, cx, "transom-vault"),
            s3_access_key: field(window, cx, "AKID..."),
            s3_secret: secret_field(window, cx, rust_i18n::t!("sync.secret_hint").as_ref()),
            directory: field(window, cx, DEFAULT_SYNC_DIRECTORY),
            passphrase: secret_field(window, cx, rust_i18n::t!("sync.passphrase_hint").as_ref()),
            git_remote: field(window, cx, rust_i18n::t!("sync.git_remote_hint").as_ref()),
            git_branch: field(window, cx, "transom-sync"),
            git_interval: field(window, cx, "0"),
            git_token: secret_field(window, cx, rust_i18n::t!("sync.git_token_hint").as_ref()),
        }
    }
}

impl AppShell {
    /// 打开设置弹窗时回填云同步表单；秘密一律留空。
    pub(crate) fn fill_sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sync = self.settings.sync.clone();
        let form = &self.settings_modal.sync_form;
        let mut set = |input: &Entity<InputState>, value: &str| {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        };
        let directory = || {
            sync.endpoint
                .as_ref()
                .map(|endpoint| endpoint.directory().to_string())
                .unwrap_or_else(|| DEFAULT_SYNC_DIRECTORY.to_string())
        };
        if let Some(git) = &sync.git {
            set(&form.git_remote, &git.remote);
            set(&form.git_branch, &git.branch);
            set(&form.git_interval, &git.interval_minutes.to_string());
        } else {
            set(&form.git_branch, "transom-sync");
            set(&form.git_interval, "0");
        }
        match &sync.endpoint {
            Some(SyncEndpoint::Webdav {
                url,
                username,
                directory,
            }) => {
                set(&form.webdav_url, url);
                set(&form.webdav_username, username);
                set(&form.directory, directory);
            }
            Some(SyncEndpoint::S3 {
                endpoint,
                region,
                bucket,
                access_key,
                directory,
            }) => {
                set(&form.s3_endpoint, endpoint);
                set(&form.s3_region, region);
                set(&form.s3_bucket, bucket);
                set(&form.s3_access_key, access_key);
                set(&form.directory, directory);
            }
            None => {
                set(&form.directory, &directory());
            }
        }
    }

    /// 从输入框汇总当前云同步配置（不落盘）。
    ///
    /// 端点类型以设置内存态为准（用户在页上点选），字段值取输入框现状，
    /// 因此"填完直接点上传"无需先保存；汇总后再做一次清洗兜底。
    pub(crate) fn collect_sync_settings(&self, cx: &App) -> SyncSettings {
        let form = &self.settings_modal.sync_form;
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let endpoint = match self.settings.sync.endpoint.clone() {
            Some(SyncEndpoint::Webdav { .. }) => Some(SyncEndpoint::Webdav {
                url: read(&form.webdav_url),
                username: read(&form.webdav_username),
                directory: read(&form.directory),
            }),
            Some(SyncEndpoint::S3 { .. }) => Some(SyncEndpoint::S3 {
                endpoint: read(&form.s3_endpoint),
                region: read(&form.s3_region),
                bucket: read(&form.s3_bucket),
                access_key: read(&form.s3_access_key),
                directory: read(&form.directory),
            }),
            None => None,
        };
        let mut sync = self.settings.sync.clone();
        sync.endpoint = endpoint.and_then(SyncEndpoint::sanitized);
        let git_remote = read(&form.git_remote);
        sync.git = if git_remote.is_empty() {
            None
        } else {
            Some(
                GitSyncSettings {
                    remote: git_remote,
                    branch: read(&form.git_branch),
                    interval_minutes: read(&form.git_interval).parse::<u64>().unwrap_or(0),
                }
                .sanitized(),
            )
            .flatten()
        };
        sync
    }

    /// 把非空的凭据输入写入钥匙串并清空输入框。
    ///
    /// 只保存与当前端点类型匹配的凭据：WebDAV 密码与 S3 Secret Key 共用
    /// 同一槽位（同一时刻只有一个端点生效），同时写两个会互相覆盖。
    ///
    /// 返回 `false` 表示校验或钥匙串写入失败，调用方应中止本次动作。
    pub(crate) fn persist_sync_credentials(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let form = &self.settings_modal.sync_form;
        let is_webdav = matches!(
            self.settings.sync.endpoint,
            Some(SyncEndpoint::Webdav { .. })
        );
        let credential = if is_webdav {
            form.webdav_password.read(cx).value().trim().to_string()
        } else {
            form.s3_secret.read(cx).value().trim().to_string()
        };
        let passphrase = form.passphrase.read(cx).value().trim().to_string();

        if !credential.is_empty() {
            let key = SecretKey::cloud_sync_credential();
            match self.services.secret_store.set(&key, &credential) {
                Ok(()) => {
                    let cleared = if is_webdav {
                        &form.webdav_password
                    } else {
                        &form.s3_secret
                    };
                    cleared.update(cx, |state, cx| state.set_value("", window, cx));
                }
                Err(error) => {
                    self.error = Some(localized_error(&error));
                    return false;
                }
            }
        }
        let git_token = form.git_token.read(cx).value().trim().to_string();
        if !git_token.is_empty() {
            let key = SecretKey::git_sync_token();
            match self.services.secret_store.set(&key, &git_token) {
                Ok(()) => form
                    .git_token
                    .update(cx, |state, cx| state.set_value("", window, cx)),
                Err(error) => {
                    self.error = Some(localized_error(&error));
                    return false;
                }
            }
        }
        if !passphrase.is_empty() {
            if passphrase.chars().count() < 8 {
                self.error = Some(rust_i18n::t!("backup.error_short").to_string());
                return false;
            }
            let key = SecretKey::cloud_sync_passphrase();
            match self.services.secret_store.set(&key, &passphrase) {
                Ok(()) => {
                    form.passphrase
                        .update(cx, |state, cx| state.set_value("", window, cx));
                }
                Err(error) => {
                    self.error = Some(localized_error(&error));
                    return false;
                }
            }
        }
        true
    }

    /// 立即执行私有 Git 仓库同步：先落凭据，再把目录快照交给控制器。
    pub(crate) fn start_git_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.persist_sync_credentials(window, cx) {
            cx.notify();
            return;
        }
        let settings = self.collect_sync_settings(cx);
        if settings.git.is_none() {
            self.error = Some(rust_i18n::t!("sync.git_no_config").to_string());
            cx.notify();
            return;
        }
        self.send_git_sync(settings, cx);
    }

    /// 定时同步入口：秘密已经在钥匙串里，不再触碰设置表单。
    pub(crate) fn start_git_sync_scheduled(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.sync.clone();
        if settings.git.is_none() {
            return;
        }
        self.send_git_sync(settings, cx);
    }

    fn send_git_sync(&mut self, settings: SyncSettings, cx: &mut Context<Self>) {
        self.status = Some(rust_i18n::t!("sync.git_running").to_string());
        if !self.send_command(AppCommand::GitSync {
            settings,
            catalog: self.catalog.clone(),
        }) {
            self.error = Some(rust_i18n::t!("sync.git_no_config").to_string());
        }
        cx.notify();
    }

    /// Git 同步完成：整体替换本地目录并刷新已打开标签。
    pub(crate) fn handle_git_synced(
        &mut self,
        result: Result<GitSyncResult, AppError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(result) => {
                let report = result.report;
                self.catalog = result.catalog;
                self.catalog.normalize();
                self.persist_catalog();
                self.refresh_tab_sessions();
                self.settings.sync.last_pull_at = Some(result.at);
                self.status = Some(
                    rust_i18n::t!(
                        "sync.git_done",
                        added = report.sessions_added,
                        updated = report.sessions_updated,
                        conflicts = report.conflicts
                    )
                    .to_string(),
                );
                self.error = None;
            }
            Err(error) => {
                self.error = Some(localized_error(&error));
            }
        }
        cx.notify();
    }

    /// 立即上传：先落凭据，再以输入框现状发起云端推送。
    pub(crate) fn start_cloud_sync_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.persist_sync_credentials(window, cx) {
            cx.notify();
            return;
        }
        let settings = self.collect_sync_settings(cx);
        if settings.endpoint.is_none() {
            self.error = Some(rust_i18n::t!("sync.error_no_endpoint").to_string());
            cx.notify();
            return;
        }
        let catalog = self.catalog.clone();
        if !self.send_command(AppCommand::CloudSyncPush { settings, catalog }) {
            return;
        }
        self.status = Some(rust_i18n::t!("sync.push_started").to_string());
        cx.notify();
    }

    /// 立即下载：流程同上传，远端文档由事件回流的合并动作接手。
    pub(crate) fn start_cloud_sync_pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.persist_sync_credentials(window, cx) {
            cx.notify();
            return;
        }
        let settings = self.collect_sync_settings(cx);
        if settings.endpoint.is_none() {
            self.error = Some(rust_i18n::t!("sync.error_no_endpoint").to_string());
            cx.notify();
            return;
        }
        if !self.send_command(AppCommand::CloudSyncPull { settings }) {
            return;
        }
        self.status = Some(rust_i18n::t!("sync.pull_started").to_string());
        cx.notify();
    }

    /// 上传完成：回填时间戳并落盘设置。
    pub(crate) fn handle_cloud_sync_pushed(
        &mut self,
        result: Result<SyncPushReport, AppError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(report) => {
                self.settings.sync.last_push_at = Some(report.at);
                if let Err(error) = self.services.settings_store.save(&self.settings) {
                    self.error = Some(localized_error(&error));
                }
                self.status = Some(
                    rust_i18n::t!(
                        "sync.pushed",
                        sessions = report.sessions,
                        kb = (report.bytes / 1024).max(1)
                    )
                    .to_string(),
                );
            }
            Err(error) => {
                self.error = Some(localized_error(&error));
            }
        }
        cx.notify();
    }

    /// 下载完成：先写本机安全副本，再在后台写回口令并合并目录。
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

        let backup = self.services.backup.clone();
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
        self._sync_pull_task = Some(task);
    }

    /// 把合并前的本机目录导出为明文安全副本；失败不阻断合并，只记日志。
    fn write_sync_safety_copy(&self) -> Option<PathBuf> {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = transom_core::paths::config_dir().join(format!("sync-safety-{timestamp}.json"));
        let options = BackupExportOptions::default();
        match self.services.backup.export(&path, &self.catalog, &options) {
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
        let summary = self.catalog.merge(catalog);
        let saved = self.persist_catalog();
        self.refresh_tab_sessions();
        self.settings.sync.last_pull_at = Some(chrono::Utc::now().timestamp());
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        }

        let mut message = rust_i18n::t!(
            "sync.pulled",
            added = summary.added_sessions,
            updated = summary.updated_sessions,
            folders = summary.added_folders
        )
        .to_string();
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
            self.status = Some(message);
        }
        self._sync_pull_task = None;
        cx.notify();
    }

    /// 同步端点上还没有备份：首次拉取属预期状态，按提示而非错误展示。
    pub(crate) fn handle_cloud_sync_missing(&mut self, cx: &mut Context<Self>) {
        self.status = Some(rust_i18n::t!("sync.missing_remote").to_string());
        cx.notify();
    }
}
