//! GPUI application shell, views, localization, and key bindings.
rust_i18n::i18n!("locales", fallback = "en");

mod activity_bar;
mod app;
mod app_render;
mod backup;
mod batch;
mod chrome;
mod connection;
mod events;
mod forward;
mod keymap;
mod keys;
mod modals;
mod notify;
mod palette;
mod pane_tree;
mod session_form;
mod session_form_serial;
mod session_form_view;
mod session_serial;
mod settings;
mod sftp;
mod shortcuts;
mod sidebar;
mod snippets;
mod split;
mod startup;
mod status_bar;
mod tabs;
mod terminal_search;
mod theme;
mod window_state;
mod workspace;

pub use app::AppShell;

use gpui_kit::component as gpui_component;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{
    AppCommand, AppEvent, BackupService, KnownHostsStore, SecretStore, SerialPortProvider,
    SessionStore, SettingsStore,
};

#[derive(Clone)]
pub struct UiServices {
    pub session_store: Arc<dyn SessionStore>,
    pub settings_store: Arc<dyn SettingsStore>,
    pub secret_store: Arc<dyn SecretStore>,
    pub known_hosts: Arc<dyn KnownHostsStore>,
    pub serial_ports: Arc<dyn SerialPortProvider>,
    pub backup: Arc<dyn BackupService>,
    pub command_tx: flume::Sender<AppCommand>,
    pub event_rx: flume::Receiver<AppEvent>,
}

pub fn run(services: UiServices) {
    let settings = services.settings_store.load().unwrap_or_default();
    let language = settings.language.clone();

    application().with_assets(assets::AllAssets).run(move |cx| {
        rust_i18n::extend!(gpui_component);
        init(cx);
        gpui_component::set_locale(&language);
        // 主题注入必须晚于 init（注册表/全局 Theme 由它建立）、早于 open_window
        // （首帧即用目标主题渲染，避免启动闪一次默认浅色）。
        theme::register_builtin(cx);
        theme::apply_appearance(&settings.appearance, cx);

        let window_bounds = {
            let width = if settings.window.window_width > 400.0 {
                settings.window.window_width
            } else {
                1280.0
            };
            let height = if settings.window.window_height > 300.0 {
                settings.window.window_height
            } else {
                800.0
            };
            let origin =
                if let (Some(x), Some(y)) = (settings.window.window_x, settings.window.window_y) {
                    point(px(x), px(y))
                } else {
                    point(px(80.0), px(80.0))
                };
            Some(WindowBounds::Windowed(Bounds {
                origin,
                size: size(px(width), px(height)),
            }))
        };

        // 密度缩放系数：标准档 16px 为 1.0，紧凑档 <1、宽松档 >1。
        let density_scale = settings.appearance.density.font_size() / 16.0;
        let options = WindowOptions {
            window_bounds,
            // 右侧 SFTP/文件面板是固定宽度列，窗口窄到小于各固定列与主面板
            // 最小宽度之和时，面板右边缘（含关闭按钮）会被窗口裁掉。
            // 900x560（标准档）留出默认侧栏 + 主面板最小宽度 + 双栏 SFTP 最小
            // 宽度的余量。布局结构件全部以 rem 表达，最小窗口按密度基准等比
            // 缩放，宽松档下固定 900px 会把这些列挤出窗口。
            window_min_size: Some(size(px(900.0 * density_scale), px(560.0 * density_scale))),
            titlebar: Some(TitlebarOptions {
                title: Some("Transom".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            cx.new(|cx| AppShell::new(window, cx, services.clone(), settings.clone()))
        })
        .expect("failed to open window");
    });
}
