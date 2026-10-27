mod controller;
#[cfg(unix)]
mod deep_link;
mod logging;

use controller::Controller;
use std::sync::Arc;
use transom_core::{
    AppEvent, BackupService, DeepLinkRequest, KnownHostsStore, LogLevel, SecretStore,
    SerialPortProvider, SessionStore, SettingsStore,
};
use transom_net::SystemSerialPortProvider;
use transom_store::{
    CatalogBackupService, JsonHistoryStore, JsonKnownHostsStore, JsonSessionStore,
    JsonSettingsStore, KeyringSecretStore, paths,
};

fn main() -> anyhow::Result<()> {
    // 日志必须最先装配：后面的单实例交接、存储建目录与控制器启动都要留痕。
    //
    // 级别取环境变量覆盖，否则退回默认档 `info`；设置页里配的级别由界面读到配置后
    // 立即补上（`transom_ui::run` 里那一次 `log::set_max_level`）。不在这里先读一遍
    // 配置文件：那会让启动路径多加载一次设置，且与界面侧的加载点各写一份容错逻辑。
    //
    // 装配失败只可能是重复装 logger（本进程不会发生），此时降级为"没有日志"继续启动，
    // 绝不能因为日志把应用关在外面。
    if let Err(error) = logging::init(logging::resolve_level(LogLevel::default())) {
        eprintln!("日志系统装配失败，本次运行只有标准错误输出，原因: {error}");
    }

    let startup = startup_request();

    // 深链与单实例判定早于一切重量级初始化（钥匙串走 D-Bus 可能挂起秒级）：
    // 已有实例在跑时本进程只负责转交请求，不该建立存储、控制器与窗口。
    // `None` 即"请求已交接"，进程直接退出。
    #[cfg(unix)]
    let Some(primary) = deep_link::acquire_primary(&startup)? else {
        log::info!("唤起请求已转交给正在运行的实例，本次启动结束");
        return Ok(());
    };

    let known_hosts: Arc<dyn KnownHostsStore> =
        Arc::new(JsonKnownHostsStore::new(paths::known_hosts_path())?);
    let session_store: Arc<dyn SessionStore> =
        Arc::new(JsonSessionStore::new(paths::sessions_path()));
    let settings_store: Arc<dyn SettingsStore> =
        Arc::new(JsonSettingsStore::new(paths::settings_path()));
    let secret_store: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new("transom"));
    let serial_ports: Arc<dyn SerialPortProvider> = Arc::new(SystemSerialPortProvider);
    let backup: Arc<dyn BackupService> = Arc::new(CatalogBackupService::new(secret_store.clone()));
    let history: Arc<dyn transom_core::HistoryStore> =
        Arc::new(JsonHistoryStore::new(paths::history_path()));

    let controller = Controller::start(
        known_hosts.clone(),
        secret_store.clone(),
        settings_store.clone(),
    )?;

    // 监听句柄保留在此直到进程结束：随控制器停机，不做脱缰派生。
    #[cfg(unix)]
    let _deep_link = primary.serve(controller.runtime(), controller.event_sender());

    deliver_startup_requests(&controller, startup);

    let services = transom_ui::UiServices {
        session_store,
        settings_store,
        secret_store,
        known_hosts,
        serial_ports,
        backup,
        history,
        command_tx: controller.command_sender(),
        event_rx: controller.event_receiver(),
    };

    transom_ui::run(services);
    drop(controller);
    Ok(())
}

/// 启动参数解析出的唤起请求（`ssh://` 地址 + 可选目录）。
///
/// 非 Unix 平台返回空请求：macOS/Windows 由平台层派发 URL 事件，这里若也
/// 接管会把同一条地址连两次。
fn startup_request() -> StartupRequest {
    #[cfg(unix)]
    {
        deep_link::request_from_args(std::env::args())
    }
    #[cfg(not(unix))]
    {
        StartupRequest::default()
    }
}

/// 跨平台统一的启动请求形态：Unix 复用深链模块的解析结果，其余平台恒为空。
#[cfg(unix)]
type StartupRequest = deep_link::LaunchRequest;
#[cfg(not(unix))]
#[derive(Default)]
struct StartupRequest;

impl StartupRequest {
    /// 把冷启动请求按到达顺序展开成事件，与运行期唤起共用同一条投递路径：
    /// 界面事件泵启动后顺序消费，不必为"带参数启动"另开入口。
    #[cfg(unix)]
    fn into_events(self) -> Vec<AppEvent> {
        let mut events: Vec<AppEvent> = self
            .urls
            .into_iter()
            .map(|url| AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            })
            .collect();
        if let Some(path) = self.dir {
            events.push(AppEvent::DeepLink {
                request: DeepLinkRequest::Dir { path },
            });
        }
        events
    }

    #[cfg(not(unix))]
    fn into_events(self) -> Vec<AppEvent> {
        Vec::new()
    }
}

/// 把冷启动带来的请求投进事件总线。
fn deliver_startup_requests(controller: &Controller, startup: StartupRequest) {
    let event_tx = controller.event_sender();
    for event in startup.into_events() {
        if let Err(error) = event_tx.try_send(event) {
            log::error!("启动参数中的唤起请求未能投递，已丢弃，原因: {error}");
        }
    }
}
