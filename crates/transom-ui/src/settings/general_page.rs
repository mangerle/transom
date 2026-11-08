//! 偏好设置中的通用页：语言、通知与声音、桌面集成、数据备份。

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::palette::Palette;
#[cfg(target_os = "linux")]
use crate::palette::localized_error;
use crate::settings::ProxyProtocol;
use crate::settings::{SettingsEvent, SettingsView};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{BellStyle, BlockViewStyle};

impl SettingsView {
    /// 语言组：跟随系统、中文、英文三档。
    ///
    /// 空字符串表示跟随系统（新建配置的默认值）；两个语言名按自身语言书写，
    /// 刻意不参与界面语言切换。
    pub(crate) fn render_settings_language(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = app_settings(cx).language.clone();
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "app.language",
                Some("settings.language_desc"),
                Self::setting_dropdown(
                    "dropdown-language",
                    current,
                    vec![
                        (
                            String::new(),
                            rust_i18n::t!("settings.language_system").to_string(),
                        ),
                        ("zh-CN".to_string(), "中文".to_string()),
                        ("en".to_string(), "English".to_string()),
                    ],
                    |this, language, cx| this.set_language(language, cx),
                    cx,
                ),
                p,
            ))
            .into_any_element()
    }

    /// 通知与声音组：终端响铃、桌面通知与通知阈值。
    pub(crate) fn render_settings_notify(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_bell = app_settings(cx).window.bell_style;
        let stepper = Self::setting_stepper(&self.state.notify_threshold, px(110.0));
        let bell_options = vec![
            (
                "bell-none",
                rust_i18n::t!("settings.bell_none").to_string(),
                BellStyle::None,
            ),
            (
                "bell-visual",
                rust_i18n::t!("settings.bell_visual").to_string(),
                BellStyle::Visual,
            ),
            (
                "bell-audible",
                rust_i18n::t!("settings.bell_audible").to_string(),
                BellStyle::Audible,
            ),
        ];
        let bell_dropdown = Self::setting_dropdown(
            "dropdown-bell-style",
            current_bell,
            bell_options
                .into_iter()
                .map(|(_, label, val)| (val, label))
                .collect(),
            |_this, style, cx| {
                update_app_settings(cx, |s| s.window.bell_style = style);
                cx.notify();
            },
            cx,
        );

        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.bell_style",
                Some("settings.bell_style_desc"),
                bell_dropdown,
                p,
            ))
            .child(crate::widgets::setting_row(
                "notify.desktop_switch",
                Some("settings.desktop_notifications_desc"),
                Switch::new("toggle-desktop-notifications")
                    .checked(app_settings(cx).window.desktop_notifications)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.window.desktop_notifications = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "notify.threshold",
                Some("notify.threshold_note"),
                stepper,
                p,
            ))
            .into_any_element()
    }

    /// 出站代理组：全局 SOCKS5 / HTTP CONNECT 出口。
    ///
    /// 会话未单独指定代理时跟随这里的配置；代理口令输入框为掩码，
    /// 保存时随配置落盘，界面与日志都不会回显明文。
    pub(crate) fn render_settings_proxy(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = self.state.proxy.kind;
        let host = self.state.proxy.host.clone();
        let port = self.state.proxy.port.clone();
        let username = self.state.proxy.username.clone();
        let password = self.state.proxy.password.clone();
        // 代理模式与全页其它枚举项统一走下拉：三档选项长度不一，分段按钮
        // 长短参差且窄面板下会把整行占满，下拉的控件宽度稳定、靠右对齐。
        let mode_control = Self::setting_dropdown(
            "dropdown-proxy-kind",
            kind,
            vec![
                (
                    ProxyProtocol::Direct,
                    rust_i18n::t!("form.proxy_direct").to_string(),
                ),
                (
                    ProxyProtocol::Socks5,
                    rust_i18n::t!("form.proxy_socks5").to_string(),
                ),
                (
                    ProxyProtocol::Http,
                    rust_i18n::t!("form.proxy_http").to_string(),
                ),
            ],
            |this, protocol, cx| {
                this.state.proxy.kind = protocol;
                cx.notify();
            },
            cx,
        );
        let mut content = v_flex().gap_2().child(crate::widgets::setting_row(
            "settings.proxy_mode",
            Some("settings.proxy_mode_desc"),
            mode_control,
            p,
        ));
        if kind != ProxyProtocol::Direct {
            content = content
                .child(crate::widgets::setting_field_row(
                    crate::widgets::form_field(
                        rust_i18n::t!("settings.proxy_host").to_string(),
                        host,
                        p,
                    ),
                    None,
                    p,
                ))
                .child(crate::widgets::setting_field_row(
                    crate::widgets::form_field(
                        rust_i18n::t!("settings.proxy_port").to_string(),
                        port,
                        p,
                    ),
                    None,
                    p,
                ))
                .child(crate::widgets::setting_field_row(
                    crate::widgets::form_field(
                        rust_i18n::t!("settings.proxy_username").to_string(),
                        username,
                        p,
                    ),
                    None,
                    p,
                ))
                .child(crate::widgets::setting_field_row(
                    crate::widgets::secret_field(
                        rust_i18n::t!("settings.proxy_password").to_string(),
                        password,
                        p,
                    ),
                    Some(rust_i18n::t!("settings.proxy_auth_hint").to_string()),
                    p,
                ));
        }
        content.into_any_element()
    }

    /// 语义块视图组：全局开关、Shell 集成注入、外观样式与自动折叠阈值。
    pub(crate) fn render_settings_block_view(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = app_settings(cx).block_view.enabled;
        let integrate = app_settings(cx).block_view.shell_integration;
        let style = app_settings(cx).block_view.style;
        let rows_input = self.state.block_collapse_rows.clone();
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.block_view_enable",
                Some("settings.block_view_enable_desc"),
                Switch::new("toggle-block-view")
                    .checked(enabled)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.block_view.enabled = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.shell_integration_enable",
                Some("settings.shell_integration_enable_desc"),
                Switch::new("toggle-shell-integration")
                    .checked(integrate)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.block_view.shell_integration = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.block_view_style",
                None,
                h_flex()
                    .gap_1()
                    .children(
                        [
                            (
                                "block-style-card",
                                "settings.block_view_card",
                                BlockViewStyle::Card,
                            ),
                            (
                                "block-style-minimal",
                                "settings.block_view_minimal",
                                BlockViewStyle::Minimal,
                            ),
                        ]
                        .into_iter()
                        .map(|(id, key, value)| {
                            Button::new(id)
                                .small()
                                .label(rust_i18n::t!(key).to_string())
                                .selected(style == value)
                                .on_click(cx.listener(move |_this, _, _, cx| {
                                    update_app_settings(cx, |s| s.block_view.style = value);
                                    cx.notify();
                                }))
                        }),
                    )
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.block_view_rows").to_string(),
                    rows_input,
                    p,
                ),
                Some(rust_i18n::t!("settings.block_view_rows_hint").to_string()),
                p,
            ))
            .into_any_element()
    }

    /// 桌面集成组：默认终端注册开关（仅 Linux）。
    ///
    /// 设计原理（Why）：这一项与其他偏好不同——它改的是系统注册而非应用内
    /// 状态，等弹窗"保存"按钮再生效会让开关与真实系统状态脱节，所以点击
    /// 即执行注册/撤销并落盘；失败时回滚开关并把错误抛给用户看。
    #[cfg(target_os = "linux")]
    pub(crate) fn render_settings_desktop_integration(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "desktop.default_terminal_switch",
                Some("desktop.default_terminal_desc"),
                Switch::new("toggle-default-terminal")
                    .checked(app_settings(cx).window.default_terminal)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        let previous = app_settings(cx).window.default_terminal;
                        update_app_settings(cx, |s| s.window.default_terminal = checked);
                        match transom_desktop::apply_default_terminal(checked) {
                            Ok(()) => {
                                let settings_snapshot = app_settings(cx).clone();
                                this.persist_settings_reported(&settings_snapshot, cx);
                            }
                            Err(error) => {
                                log::error!(
                                    "默认终端{}失败，界面已回滚，原因: {error}",
                                    if checked { "注册" } else { "撤销" }
                                );
                                update_app_settings(cx, |s| s.window.default_terminal = previous);
                                this.report_error(localized_error(&error), cx);
                            }
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 数据备份组：全量导入与导出。
    pub(crate) fn render_settings_backup(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::widgets::setting_frame(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("backup.settings_hint").to_string()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("backup-export")
                                .small()
                                .icon(IconName::Download)
                                .label(rust_i18n::t!("backup.export_action").to_string())
                                .on_click(cx.listener(|_this, _, _window, cx| {
                                    cx.emit(SettingsEvent::OpenBackupExport);
                                })),
                        )
                        .child(
                            Button::new("backup-import")
                                .small()
                                .icon(IconName::Upload)
                                .label(rust_i18n::t!("backup.import_action").to_string())
                                .on_click(cx.listener(|_this, _, _window, cx| {
                                    cx.emit(SettingsEvent::OpenBackupImport);
                                })),
                        ),
                )
                .into_any_element(),
            p,
        )
    }
}
