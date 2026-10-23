//! GPUI application shell, views, localization, and key bindings.
rust_i18n::i18n!("locales", fallback = "en");

mod app;
mod backup;
mod batch;
mod chrome;
mod connection;
mod events;
mod forward;
mod keymap;
mod keys;
mod modals;
mod palette;
mod session_form;
mod session_form_view;
mod session_serial;
mod settings;
mod sftp;
mod shortcuts;
mod sidebar;
mod split;
mod startup;
mod status_bar;
mod tabs;
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

        let options = WindowOptions {
            window_bounds,
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
