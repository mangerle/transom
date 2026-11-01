//! 云同步表单装配与凭据落盘。

use super::common::*;
use crate::app::app_settings;

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
            git_interval: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("0")
                    .min(0.0)
                    .max(1440.0)
                    .step(5.0)
            }),
            git_token: secret_field(window, cx, rust_i18n::t!("sync.git_token_hint").as_ref()),
        }
    }
}

impl AppShell {
    /// 打开设置弹窗时回填云同步表单；秘密一律留空。
    pub(crate) fn fill_sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sync = app_settings(cx).sync.clone();
        let form = &self.state.modals.settings.sync_form;
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
        let form = &self.state.modals.settings.sync_form;
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let endpoint = match app_settings(cx).sync.endpoint.clone() {
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
        let mut sync = app_settings(cx).sync.clone();
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
        let form = &self.state.modals.settings.sync_form;
        let is_webdav = matches!(
            app_settings(cx).sync.endpoint,
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
            match self.state.services.secret_store.set(&key, &credential) {
                Ok(()) => {
                    let cleared = if is_webdav {
                        &form.webdav_password
                    } else {
                        &form.s3_secret
                    };
                    cleared.update(cx, |state, cx| state.set_value("", window, cx));
                }
                Err(error) => {
                    self.state.set_error(localized_error(&error));
                    return false;
                }
            }
        }
        let git_token = form.git_token.read(cx).value().trim().to_string();
        if !git_token.is_empty() {
            let key = SecretKey::git_sync_token();
            match self.state.services.secret_store.set(&key, &git_token) {
                Ok(()) => form
                    .git_token
                    .update(cx, |state, cx| state.set_value("", window, cx)),
                Err(error) => {
                    self.state.set_error(localized_error(&error));
                    return false;
                }
            }
        }
        if !passphrase.is_empty() {
            if passphrase.chars().count() < 8 {
                self.state
                    .set_error(rust_i18n::t!("backup.error_short").to_string());
                return false;
            }
            let key = SecretKey::cloud_sync_passphrase();
            match self.state.services.secret_store.set(&key, &passphrase) {
                Ok(()) => {
                    form.passphrase
                        .update(cx, |state, cx| state.set_value("", window, cx));
                }
                Err(error) => {
                    self.state.set_error(localized_error(&error));
                    return false;
                }
            }
        }
        true
    }
}
