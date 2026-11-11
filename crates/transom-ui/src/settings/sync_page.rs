//! 偏好设置中的云同步页组：端点配置、凭据入口与手动上传下载。

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::palette::Palette;
use crate::settings::{SettingsEvent, SettingsView};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::AppSettings;
use transom_core::SyncEndpoint;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 同步页输入行：标签 + 输入框，带分割线。
///
/// 本页说明文字都跨多个字段（凭据口径、Git 权限要求），由调用方单独挂在
/// 对应的那一行上，不随辅助函数硬绑某个输入框。
fn sync_form_row(label_key: &'static str, input: Entity<InputState>, p: &Palette) -> AnyElement {
    crate::widgets::setting_field_row(
        crate::widgets::form_field(rust_i18n::t!(label_key).to_string(), input, p),
        None,
        p,
    )
}

/// 当前生效的云同步端点类型键。
///
/// 判定顺序固定为 Git 优先：Git 同步没有 `endpoint`，二者不会同时生效；
/// 写在 `impl` 外供设置实体与页面渲染共用。
pub(crate) fn sync_endpoint_kind(settings: &AppSettings) -> &'static str {
    if settings.sync.git.is_some() {
        "git"
    } else {
        match settings.sync.endpoint {
            Some(SyncEndpoint::Webdav { .. }) => "webdav",
            Some(SyncEndpoint::S3 { .. }) => "s3",
            None => "off",
        }
    }
}

impl SettingsView {
    /// 云同步组：端点选择、拓扑配置、同步口令与上传下载动作。
    ///
    /// 端点类型选中即改设置内存态（保存时落盘），字段值始终从输入框现读，
    /// 因此"填完直接点上传"不需要先按保存。
    pub(crate) fn render_settings_cloud_sync(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_kind = sync_endpoint_kind(app_settings(cx));

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

        let form = &self.state.sync_form;
        let mut content = v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
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

        content = content.child(self.render_sync_endpoint_fields(current_kind, p));

        // 只要启用了任一同步方式（WebDAV / S3 / Git），就展示同步口令
        if current_kind != "off" {
            content = content.child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("sync.passphrase").to_string(),
                    form.passphrase.clone(),
                    p,
                ),
                Some(rust_i18n::t!("sync.passphrase_hint").to_string()),
                p,
            ));
        }

        if let Some(actions) = self.render_sync_actions(current_kind, p, cx) {
            content = content.child(actions);
        }

        content.into_any_element()
    }

    /// 端点类型对应的字段组：关闭提示、WebDAV、S3 或 Git。
    fn render_sync_endpoint_fields(&self, current_kind: &str, p: &Palette) -> AnyElement {
        let form = &self.state.sync_form;
        let mut content = v_flex().gap_2();
        match current_kind {
            "off" => {
                content = content.child(crate::widgets::setting_frame(
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
                content = content.child(git_sync_fields(form, p));
            }
            _ => {}
        }

        content.into_any_element()
    }

    /// 包含密钥、状态行与上传/下载/Git 同步动作；关闭端点时返回 `None`。
    fn render_sync_actions(
        &self,
        current_kind: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if current_kind == "off" {
            return None;
        }
        let mut content = v_flex().gap_2();
        // WebDAV 或 S3 提供包含密钥开关、状态行与上传/下载操作
        if current_kind == "webdav" || current_kind == "s3" {
            content = content
                .child(crate::widgets::setting_row(
                    "sync.include_secrets",
                    Some("sync.include_secrets_desc"),
                    Switch::new("toggle-sync-include-secrets")
                        .small()
                        .checked(app_settings(cx).sync.include_secrets)
                        .on_click(cx.listener(|_this, &checked, _, cx| {
                            update_app_settings(cx, |s| s.sync.include_secrets = checked);
                            cx.notify();
                        }))
                        .into_any_element(),
                    p,
                ))
                .child(crate::widgets::setting_frame(
                    self.sync_status_row(p, app_settings(cx)),
                    p,
                ))
                .child(crate::widgets::setting_frame(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("sync-push-now")
                                .small()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::UPLOAD)).small())
                                .label(rust_i18n::t!("sync.push").to_string())
                                .on_click(cx.listener(|_this, _, _window, cx| {
                                    cx.emit(SettingsEvent::CloudSyncPush);
                                })),
                        )
                        .child(
                            Button::new("sync-pull-now")
                                .small()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                                .label(rust_i18n::t!("sync.pull").to_string())
                                .on_click(cx.listener(|_this, _, _window, cx| {
                                    cx.emit(SettingsEvent::CloudSyncPull);
                                })),
                        )
                        .into_any_element(),
                    p,
                ));
        } else if current_kind == "git" {
            content = content.child(crate::widgets::setting_frame(
                Button::new("sync-git-now")
                    .small()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::VCS)).small())
                    .label(rust_i18n::t!("sync.git_sync_now").to_string())
                    .on_click(cx.listener(|_this, _, _window, cx| {
                        cx.emit(SettingsEvent::GitSyncNow);
                    }))
                    .into_any_element(),
                p,
            ));
        }

        Some(content.into_any_element())
    }

    /// 最近一次上传/下载时间行；从未同步时显示"从未"。
    fn sync_status_row(&self, p: &Palette, settings: &AppSettings) -> AnyElement {
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
                format(settings.sync.last_push_at)
            ))
            .child(format!(
                "{}: {}",
                rust_i18n::t!("sync.last_pull"),
                format(settings.sync.last_pull_at)
            ))
            .into_any_element()
    }
}

/// Git 同步字段：远端、分支、间隔与令牌。
fn git_sync_fields(form: &crate::cloud_sync::SyncFormInputs, p: &Palette) -> AnyElement {
    let stepper = SettingsView::setting_stepper(&form.git_interval, px(110.0));
    v_flex()
        .gap_2()
        .child(sync_form_row("sync.git_remote", form.git_remote.clone(), p))
        .child(crate::widgets::setting_frame(
            h_flex()
                .w_full()
                .gap_4()
                .items_center()
                .child(div().flex_1().child(crate::widgets::form_field(
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
        .child(crate::widgets::setting_field_row(
            crate::widgets::secret_field(
                rust_i18n::t!("sync.git_token").to_string(),
                form.git_token.clone(),
                p,
            ),
            Some(rust_i18n::t!("sync.git_hint").to_string()),
            p,
        ))
        .into_any_element()
}
