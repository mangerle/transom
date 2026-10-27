//! GPUI application shell, views, localization, and key bindings.
rust_i18n::i18n!("locales", fallback = "en");

/// 应用展示元数据。构建期由 cargo 注入版本号，状态栏兜底文案与设置"关于"页共用
/// 这一份，避免版本号与仓库地址散落到多处硬编码后各自漂移。
pub(crate) const APP_NAME: &str = "Transom";
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const APP_LICENSE: &str = env!("CARGO_PKG_LICENSE");
pub(crate) const APP_REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub(crate) const APP_AUTHORS: &str = "wangle";

mod activity_bar;
mod app;
mod app_render;
mod audit;
mod audit_viewer;
mod backup;
mod batch;
mod brand;
mod broadcast;
mod cast_player;
mod chrome;
mod cloud_sync;
mod connection;
mod event_handlers;
mod events;
mod forward;
mod identity_panel;
mod keymap;
mod keys;
mod modals;
mod monitor;
mod notify;
mod palette;
mod pane_tree;
mod probe;
mod record_view;
mod session_appearance;
mod session_form;
mod session_form_appearance;
mod session_form_serial;
mod session_form_telnet;
mod session_form_view;
mod session_serial;
mod settings;
mod sftp;
mod sftp_handlers;
mod shortcuts;
mod sidebar;
mod snippets;
mod split;
mod startup;
mod status_bar;
mod tabs;
mod terminal_search;
mod theme;
mod tiling;
mod trigger_panel;
mod window_appearance;
mod window_state;
mod workspace;
mod zmodem;

pub use app::AppShell;

use gpui_kit::component as gpui_component;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{
    AppCommand, AppEvent, AppSettings, BackupService, KnownHostsStore, LogLevel, SecretStore,
    SerialPortProvider, SessionStore, SettingsStore,
};

#[derive(Clone)]
pub struct UiServices {
    pub session_store: Arc<dyn SessionStore>,
    pub settings_store: Arc<dyn SettingsStore>,
    pub secret_store: Arc<dyn SecretStore>,
    pub known_hosts: Arc<dyn KnownHostsStore>,
    pub serial_ports: Arc<dyn SerialPortProvider>,
    pub backup: Arc<dyn BackupService>,
    /// 命令历史读写（独立于会话目录的一份本地文件）。
    pub history: Arc<dyn transom_core::HistoryStore>,
    pub command_tx: flume::Sender<AppCommand>,
    pub event_rx: flume::Receiver<AppEvent>,
}

pub fn run(services: UiServices) {
    // 设置文件读不出来时不能把用户关在外面：按默认值起界面，但一定要在日志里留痕，
    // 否则"设置莫名其妙回到默认"这类报障完全没有线索可查。
    let settings = match services.settings_store.load() {
        Ok(settings) => settings,
        Err(error) => {
            log::error!("读取设置失败，本次启动按默认配置运行，原因: {error}");
            AppSettings::default()
        }
    };
    let language = settings.language.clone();
    reassert_default_terminal(&settings);
    // 应用装配日志时还不知道配置值，这里补上用户设的级别。
    //
    // 环境变量仍然优先：报障场景下 `TRANSOM_LOG=debug` 必须能盖过配置里的 `info`，
    // 否则用户改一次配置就把现场冲掉了。设置页里显式改级别走的是另一条路径
    // （直接 `log::set_max_level`），代表用户最新意图，不再被环境变量否决。
    log::set_max_level(
        LogLevel::from_env()
            .unwrap_or(settings.logs.level)
            .to_filter(),
    );

    application().with_assets(assets::AllAssets).run(move |cx| {
        rust_i18n::extend!(gpui_component);
        init(cx);
        gpui_component::set_locale(&language);
        // 主题注入必须晚于 init（注册表/全局 Theme 由它建立）、早于 open_window
        // （首帧即用目标主题渲染，避免启动闪一次默认浅色）。
        theme::register_builtin(cx);
        theme::watch_user_dir(cx);
        // 窗口外观（整体透明与毛玻璃）同样是全局值，且必须先于主题注入：
        // 主题注入时会按它折算表面色 alpha，顺序反了首帧就是实色。
        window_appearance::apply_window_appearance(&settings.window, cx);
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
            // 窗口背景按设置处理：透明或毛玻璃。
            window_background: window_appearance::background_appearance(&settings.window),
            titlebar: Some(TitlebarOptions {
                title: Some("Transom".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            // 桌面标识用于任务栏分组与 `ssh://` 处理程序回指本应用，必须与
            // packaging/linux 下 .desktop 的 StartupWMClass 保持一致。
            window.set_app_id(transom_core::paths::APP_ID);
            cx.new(|cx| AppShell::new(window, cx, services.clone(), settings.clone()))
        })
        .expect("failed to open window");
    });
}

/// 开关为"开"时校验并（仅在不一致时）重言默认终端注册。
///
/// 设计原理（Why）：注册里的 `Exec` 写的是当前可执行文件绝对路径，构建位置
/// 或安装前缀一变注册就悬空，所以每次启动比对一次；比对是三次小文件读，
/// 漂移才重写，常态启动零写入。放在 `run` 而不是 `main`：这里已经加载过
/// 设置，`main` 侧再读一遍会出现第二个加载点与另一份容错逻辑。
///
/// 失败只降级不阻断启动：桌面集成是可延后的体验，不是主流程前提。
#[cfg(target_os = "linux")]
fn reassert_default_terminal(settings: &AppSettings) {
    if !settings.window.default_terminal {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        log::warn!("无法定位当前可执行文件，默认终端注册校验跳过");
        return;
    };
    if transom_core::paths::is_dev_exe(&exe) {
        log::debug!(
            "当前运行于开发构建目录，跳过默认终端启动重言，路径: {}",
            exe.display()
        );
        return;
    }
    if transom_store::desktop::default_terminal_registered(&exe) {
        return;
    }
    match transom_store::desktop::apply_default_terminal(true) {
        Ok(()) => log::info!("默认终端注册已重言，可执行文件: {}", exe.display()),
        Err(error) => log::warn!("默认终端注册重言失败，桌面入口暂不可用，原因: {error}"),
    }
}

/// 非 Linux 平台没有默认终端规范可对账，开关本身也不在设置页展示。
#[cfg(not(target_os = "linux"))]
fn reassert_default_terminal(_settings: &AppSettings) {}
