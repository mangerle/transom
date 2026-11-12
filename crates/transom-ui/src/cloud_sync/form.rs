//! 云同步表单装配与凭据落盘。

use super::common::*;

/// 设置弹窗里云同步组的全部输入框。
///
/// 密码、Secret Key 与同步口令为掩码输入且打开弹窗时不回填：钥匙串里
/// 已有的值不回显，只有用户重新输入才会覆盖，避免旁人看到明文。
#[derive(Clone)]
pub(crate) struct SyncFormInputs {
    pub(crate) webdav_url: Entity<InputState>,
    pub(crate) webdav_username: Entity<InputState>,
    pub(crate) webdav_password: Entity<InputState>,
    pub(crate) s3_endpoint: Entity<InputState>,
    pub(crate) s3_region: Entity<InputState>,
    pub(crate) s3_bucket: Entity<InputState>,
    pub(crate) s3_access_key: Entity<InputState>,
    pub(crate) s3_secret: Entity<InputState>,
    pub(crate) s3_session_token: Entity<InputState>,
    pub(crate) directory: Entity<InputState>,
    pub(crate) device_name: Entity<InputState>,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) git_remote: Entity<InputState>,
    pub(crate) git_branch: Entity<InputState>,
    pub(crate) git_interval: Entity<InputState>,
    pub(crate) auto_pull_debounce: Entity<InputState>,
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
            s3_session_token: secret_field(
                window,
                cx,
                rust_i18n::t!("sync.session_token_hint").as_ref(),
            ),
            directory: field(window, cx, DEFAULT_SYNC_DIRECTORY),
            device_name: field(window, cx, rust_i18n::t!("sync.device_name_hint").as_ref()),
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
            auto_pull_debounce: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("60")
                    .min(5.0)
                    .max(3600.0)
                    .step(5.0)
            }),
            git_token: secret_field(window, cx, rust_i18n::t!("sync.git_token_hint").as_ref()),
        }
    }
}
