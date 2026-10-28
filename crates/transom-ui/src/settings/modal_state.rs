//! 设置弹窗状态的构造：全部输入框在此一次性建立。
//!
//! 设计原理（Why）：AppShell::new 曾内联约一百八十行的输入框构造样板
//! （回填值、占位符、掩码），与"装配应用"的其余步骤混在同一函数里，
//! 新增一个设置项要穿过整个 new 找位置。把构造收进状态类型自己的构造
//! 函数后，设置项只改本文件，AppShell::new 保持可读。

use crate::app::AppShell;
use crate::cloud_sync::SyncFormInputs;
use crate::identity::IdentityEditState;
use crate::keys::KeyWizardState;
use crate::trigger_panel::TriggerEditState;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::slider::SliderState;
use gpui_kit::*;
use transom_core::{AppSettings, AppearanceSettings, WindowBackgroundMode};

/// 窗口外观的即时快照：设置弹窗取消时用于回滚透明与背景模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowAppearanceSnapshot {
    pub(crate) transparency: u8,
    pub(crate) background: WindowBackgroundMode,
}

/// 设置弹窗内的代理协议选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyProtocol {
    Direct,
    Socks5,
    Http,
}

/// 设置弹窗的代理表单输入。
pub(crate) struct ProxyFormInputs {
    pub(crate) kind: ProxyProtocol,
    pub(crate) host: Entity<InputState>,
    pub(crate) port: Entity<InputState>,
    pub(crate) username: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
}

/// 设置弹窗的全部输入控件与临时状态。
///
/// 字段仍由各设置页直接读写：它们只在弹窗打开期间存活，且没有跨字段
/// 不变式需要维护；把控件集中在一个类型里是为了让 `AppShell` 不必平铺
/// 二十多个输入框句柄。
pub(crate) struct SettingsModalState {
    pub(crate) open: bool,
    pub(crate) key_wizard: Option<KeyWizardState>,
    pub(crate) font_family: Entity<InputState>,
    pub(crate) ui_font_family: Entity<InputState>,
    pub(crate) panel_hidden_patterns: Entity<InputState>,
    pub(crate) max_concurrent_transfers: Entity<InputState>,
    pub(crate) cjk_font: Entity<InputState>,
    pub(crate) font_size: Entity<InputState>,
    pub(crate) line_height: Entity<InputState>,
    pub(crate) scrollback: Entity<InputState>,
    pub(crate) word_separators: Entity<InputState>,
    pub(crate) notify_threshold: Entity<InputState>,
    pub(crate) block_collapse_rows: Entity<InputState>,
    pub(crate) terminal_key_bindings: Entity<TextareaState>,
    pub(crate) highlight_keywords: Entity<InputState>,
    pub(crate) window_transparency: Entity<SliderState>,
    pub(crate) window_appearance_snapshot: Option<WindowAppearanceSnapshot>,
    pub(crate) appearance_snapshot: Option<AppearanceSettings>,
    pub(crate) sync_form: SyncFormInputs,
    pub(crate) proxy: ProxyFormInputs,
    pub(crate) identity_edit: Option<IdentityEditState>,
    pub(crate) trigger_edit: Option<TriggerEditState>,
}

impl SettingsModalState {
    /// 按当前设置回填构造全部输入框与滑块。
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<AppShell>,
        settings: &AppSettings,
    ) -> Self {
        let settings_font_family_val = settings
            .terminal_profile
            .font_family
            .clone()
            .unwrap_or_default();
        let settings_font_family = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.font_family_hint").to_string())
                .default_value(settings_font_family_val)
        });
        let settings_ui_font_val = settings
            .appearance
            .ui_font_family
            .clone()
            .unwrap_or_default();
        let settings_ui_font_family = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.ui_font_family_hint").to_string())
                .default_value(settings_ui_font_val)
        });
        let settings_panel_hidden = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.panel_hidden_hint").to_string())
                .default_value(settings.sftp.panel_hidden_patterns.clone())
        });
        let settings_max_concurrent = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("3")
                .default_value(settings.sftp.max_concurrent_transfers.to_string())
        });
        let settings_cjk_font_val = settings
            .terminal_profile
            .cjk_font_family
            .clone()
            .unwrap_or_default();
        let settings_cjk_font = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.cjk_font_hint").to_string())
                .default_value(settings_cjk_font_val)
        });
        let font_size_val = settings.terminal_profile.font_size.to_string();
        let settings_font_size = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("14")
                .default_value(&font_size_val)
        });
        let line_height_val = settings.terminal_profile.line_height.to_string();
        let settings_line_height = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1.2")
                .default_value(&line_height_val)
        });
        // 分隔符必须原样回填：空格本身就是合法分隔符，任何 trim 都会吃掉用户意图。
        let settings_word_separators = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.word_separators_hint").to_string())
                .default_value(settings.terminal_profile.word_separators.clone())
        });
        let settings_highlight_keywords = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.highlight_keywords_hint").to_string())
                .default_value(settings.terminal_profile.highlight.keywords_joined())
        });
        let notify_threshold_val = settings.window.notify_threshold_secs.to_string();
        let settings_notify_threshold = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("10")
                .default_value(notify_threshold_val)
        });
        let block_collapse_rows_val = settings.block_view.auto_collapse_rows.to_string();
        let settings_block_collapse_rows = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("150")
                .default_value(block_collapse_rows_val)
        });
        let scrollback_val = settings.terminal_profile.scrollback.to_string();
        let settings_scrollback = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("10000")
                .default_value(&scrollback_val)
        });
        let key_bindings_text =
            transom_core::TerminalKeyBinding::to_multiline(&settings.terminal_profile.key_bindings);
        let settings_terminal_key_bindings = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("ctrl-shift-left = \\e[1;6D")
                .default_value(key_bindings_text)
        });
        let (proxy_kind, proxy_host_val, proxy_port_val, proxy_user_val, proxy_pass_val) =
            match &settings.proxy {
                transom_core::OutboundProxy::Direct => (
                    ProxyProtocol::Direct,
                    String::new(),
                    "1080".to_string(),
                    String::new(),
                    String::new(),
                ),
                transom_core::OutboundProxy::Socks5 { host, port, auth } => (
                    ProxyProtocol::Socks5,
                    host.clone(),
                    port.to_string(),
                    auth.as_ref()
                        .map_or_else(String::new, |auth| auth.username.clone()),
                    auth.as_ref()
                        .map_or_else(String::new, |auth| auth.password.clone()),
                ),
                transom_core::OutboundProxy::Http { host, port, auth } => (
                    ProxyProtocol::Http,
                    host.clone(),
                    port.to_string(),
                    auth.as_ref()
                        .map_or_else(String::new, |auth| auth.username.clone()),
                    auth.as_ref()
                        .map_or_else(String::new, |auth| auth.password.clone()),
                ),
            };
        let proxy = ProxyFormInputs {
            kind: proxy_kind,
            host: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("127.0.0.1")
                    .default_value(proxy_host_val)
            }),
            port: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("1080")
                    .default_value(proxy_port_val)
            }),
            username: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("settings.proxy_username_hint").to_string())
                    .default_value(proxy_user_val)
            }),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(rust_i18n::t!("settings.proxy_password_hint").to_string())
                    .default_value(proxy_pass_val)
            }),
        };
        // 窗口整体透明度是连续量：滑块比输入框直观，拖动时还能即时预览。
        let window_transparency = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(f32::from(settings.window.window_transparency))
        });
        // 云同步表单：端点字段回填设置值，凭据掩码框恒为空。
        let sync_form = SyncFormInputs::new(window, cx);

        Self {
            open: false,
            proxy,
            key_wizard: None,
            font_family: settings_font_family,
            ui_font_family: settings_ui_font_family,
            panel_hidden_patterns: settings_panel_hidden,
            max_concurrent_transfers: settings_max_concurrent,
            cjk_font: settings_cjk_font,
            font_size: settings_font_size,
            line_height: settings_line_height,
            scrollback: settings_scrollback,
            word_separators: settings_word_separators,
            notify_threshold: settings_notify_threshold,
            block_collapse_rows: settings_block_collapse_rows,
            terminal_key_bindings: settings_terminal_key_bindings,
            highlight_keywords: settings_highlight_keywords,
            window_transparency,
            window_appearance_snapshot: None,
            appearance_snapshot: None,
            identity_edit: None,
            trigger_edit: None,
            sync_form,
        }
    }
}
