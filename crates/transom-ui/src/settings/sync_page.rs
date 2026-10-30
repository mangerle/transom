//! 偏好设置中的云同步页组：端点配置、凭据入口与手动上传下载。

use crate::app::AppShell;
use crate::cloud_sync::DEFAULT_SYNC_DIRECTORY;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::SyncEndpoint;

/// 同步页输入行：标签 + 输入框，带分割线。
///
/// 本页说明文字都跨多个字段（凭据口径、Git 权限要求），由调用方单独挂在
/// 对应的那一行上，不随辅助函数硬绑某个输入框。
fn sync_form_row(label_key: &'static str, input: Entity<InputState>, p: &Palette) -> AnyElement {
    AppShell::setting_field_row(
        AppShell::form_field(rust_i18n::t!(label_key).to_string(), input, p),
        None,
        p,
    )
}

impl AppShell {
    /// 云同步组：端点选择、拓扑配置、同步口令与上传下载动作。
    ///
    /// 端点类型选中即改设置内存态（保存时落盘），字段值始终从输入框现读，
    /// 因此"填完直接点上传"不需要先按保存。
    pub(crate) fn render_settings_cloud_sync(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_kind = self.sync_endpoint_kind();

        // 四种端点类型走下拉：与全页其它枚举项一致，长标签（WebDAV、Git 仓库）
        // 不会把整行占满；切换要回填表单输入框，所以用带窗口句柄的变体。
        let endpoint_control = Self::setting_dropdown_with_window(
            "dropdown-sync-endpoint",
            current_kind,
            vec![
                ("off", rust_i18n::t!("sync.endpoint_off").to_string()),
                ("webdav", rust_i18n::t!("sync.endpoint_webdav").to_string()),
                ("s3", rust_i18n::t!("sync.endpoint_s3").to_string()),
                ("git", rust_i18n::t!("sync.endpoint_git").to_string()),
            ],
            |this, kind, window, cx| this.switch_sync_endpoint(kind, window, cx),
            cx,
        );

        let form = &self.modals.settings.sync_form;
        let mut content = v_flex()
            .gap_2()
            .child(Self::setting_row(
                "sync.endpoint",
                Some("sync.endpoint_desc"),
                endpoint_control,
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sync.credential_hint").to_string()),
            );

        match current_kind {
            "off" => {
                content = content.child(Self::setting_frame(
                    div()
                        .w_full()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("sync.sync_disabled_hint").to_string())
                        .into_any_element(),
                    p,
                ));
            }
            "webdav" => {
                content = content
                    .child(sync_form_row("sync.webdav_url", form.webdav_url.clone(), p))
                    .child(sync_form_row(
                        "sync.username",
                        form.webdav_username.clone(),
                        p,
                    ))
                    .child(sync_form_row(
                        "sync.password",
                        form.webdav_password.clone(),
                        p,
                    ))
                    .child(sync_form_row("sync.directory", form.directory.clone(), p));
            }
            "s3" => {
                content = content
                    .child(sync_form_row(
                        "sync.s3_endpoint",
                        form.s3_endpoint.clone(),
                        p,
                    ))
                    .child(sync_form_row("sync.region", form.s3_region.clone(), p))
                    .child(sync_form_row("sync.bucket", form.s3_bucket.clone(), p))
                    .child(sync_form_row(
                        "sync.access_key",
                        form.s3_access_key.clone(),
                        p,
                    ))
                    .child(sync_form_row("sync.secret", form.s3_secret.clone(), p))
                    .child(sync_form_row("sync.directory", form.directory.clone(), p));
            }
            "git" => {
                let stepper = Self::setting_stepper(&form.git_interval, px(110.0));
                content = content
                    .child(sync_form_row("sync.git_remote", form.git_remote.clone(), p))
                    .child(Self::setting_frame(
                        h_flex()
                            .w_full()
                            .gap_4()
                            .items_center()
                            .child(div().flex_1().child(Self::form_field(
                                rust_i18n::t!("sync.git_branch").to_string(),
                                form.git_branch.clone(),
                                p,
                            )))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.muted_foreground)
                                            .child(rust_i18n::t!("sync.git_interval").to_string()),
                                    )
                                    .child(stepper),
                            )
                            .into_any_element(),
                        p,
                    ))
                    .child(Self::setting_field_row(
                        Self::secret_field(
                            rust_i18n::t!("sync.git_token").to_string(),
                            form.git_token.clone(),
                            p,
                        ),
                        Some(rust_i18n::t!("sync.git_hint").to_string()),
                        p,
                    ));
            }
            _ => {}
        }

        // 只要启用了任一同步方式（WebDAV / S3 / Git），就展示同步口令
        if current_kind != "off" {
            content = content.child(Self::setting_field_row(
                Self::form_field(
                    rust_i18n::t!("sync.passphrase").to_string(),
                    form.passphrase.clone(),
                    p,
                ),
                Some(rust_i18n::t!("sync.passphrase_hint").to_string()),
                p,
            ));
        }

        // WebDAV 或 S3 提供包含密钥开关、状态行与上传/下载操作
        if current_kind == "webdav" || current_kind == "s3" {
            content = content
                .child(Self::setting_row(
                    "sync.include_secrets",
                    Some("sync.include_secrets_desc"),
                    Switch::new("toggle-sync-include-secrets")
                        .small()
                        .checked(self.settings.sync.include_secrets)
                        .on_click(cx.listener(|this, &checked, _, cx| {
                            this.settings.sync.include_secrets = checked;
                            cx.notify();
                        }))
                        .into_any_element(),
                    p,
                ))
                .child(Self::setting_frame(self.sync_status_row(p), p))
                .child(Self::setting_frame(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("sync-push-now")
                                .small()
                                .icon(IconName::Upload)
                                .label(rust_i18n::t!("sync.push").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_cloud_sync_push(window, cx);
                                })),
                        )
                        .child(
                            Button::new("sync-pull-now")
                                .small()
                                .icon(IconName::Download)
                                .label(rust_i18n::t!("sync.pull").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_cloud_sync_pull(window, cx);
                                })),
                        )
                        .into_any_element(),
                    p,
                ));
        } else if current_kind == "git" {
            content = content.child(Self::setting_frame(
                Button::new("sync-git-now")
                    .small()
                    .icon(IconName::GitBranch)
                    .label(rust_i18n::t!("sync.git_sync_now").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_git_sync(window, cx);
                    }))
                    .into_any_element(),
                p,
            ));
        }

        content.into_any_element()
    }

    /// 最近一次上传/下载时间行；从未同步时显示"从未"。
    fn sync_status_row(&self, p: &Palette) -> AnyElement {
        let format = |at: Option<i64>| {
            at.and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|| rust_i18n::t!("sync.never").to_string())
        };
        h_flex()
            .gap_4()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(format!(
                "{}: {}",
                rust_i18n::t!("sync.last_push"),
                format(self.settings.sync.last_push_at)
            ))
            .child(format!(
                "{}: {}",
                rust_i18n::t!("sync.last_pull"),
                format(self.settings.sync.last_pull_at)
            ))
            .into_any_element()
    }

    /// 当前生效的同步端点类型：Git 优先（`git` 有值时端点字段恒为空）。
    fn sync_endpoint_kind(&self) -> &'static str {
        if self.settings.sync.git.is_some() {
            "git"
        } else {
            match self.settings.sync.endpoint {
                Some(SyncEndpoint::Webdav { .. }) => "webdav",
                Some(SyncEndpoint::S3 { .. }) => "s3",
                None => "off",
            }
        }
    }

    /// 切换端点类型：切到未配置的类型时置默认拓扑并回填输入框。
    ///
    /// 选中当前类型直接返回：末尾的 `fill_sync_form` 会用设置值覆盖全部输入框，
    /// 不拦这一下就会把用户尚未保存的编辑冲掉；下拉里重新点当前项是常见操作，
    /// 必须幂等。
    fn switch_sync_endpoint(
        &mut self,
        kind: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind == self.sync_endpoint_kind() {
            return;
        }
        match kind {
            "off" => {
                self.settings.sync.endpoint = None;
                self.settings.sync.git = None;
                self.modals
                    .settings
                    .sync_form
                    .git_remote
                    .update(cx, |input, cx| {
                        input.set_value(String::new(), window, cx);
                    });
            }
            "webdav" => {
                self.settings.sync.git = None;
                self.modals
                    .settings
                    .sync_form
                    .git_remote
                    .update(cx, |input, cx| {
                        input.set_value(String::new(), window, cx);
                    });
                if !matches!(
                    self.settings.sync.endpoint,
                    Some(SyncEndpoint::Webdav { .. })
                ) {
                    self.settings.sync.endpoint = Some(SyncEndpoint::Webdav {
                        url: String::new(),
                        username: String::new(),
                        directory: DEFAULT_SYNC_DIRECTORY.to_string(),
                    });
                }
            }
            "s3" => {
                self.settings.sync.git = None;
                self.modals
                    .settings
                    .sync_form
                    .git_remote
                    .update(cx, |input, cx| {
                        input.set_value(String::new(), window, cx);
                    });
                if !matches!(self.settings.sync.endpoint, Some(SyncEndpoint::S3 { .. })) {
                    self.settings.sync.endpoint = Some(SyncEndpoint::S3 {
                        endpoint: String::new(),
                        region: String::new(),
                        bucket: String::new(),
                        access_key: String::new(),
                        directory: DEFAULT_SYNC_DIRECTORY.to_string(),
                    });
                }
            }
            "git" => {
                self.settings.sync.endpoint = None;
                if self.settings.sync.git.is_none() {
                    self.settings.sync.git = Some(transom_core::GitSyncSettings {
                        remote: String::new(),
                        branch: "transom-sync".to_string(),
                        interval_minutes: 0,
                    });
                }
            }
            _ => return,
        }
        self.fill_sync_form(window, cx);
        cx.notify();
    }
}
