mod controller;
#[cfg(unix)]
mod deep_link;

use controller::Controller;
use std::sync::Arc;
use transom_core::{
    AppEvent, BackupService, KnownHostsStore, SecretStore, SerialPortProvider, SessionStore,
    SettingsStore,
};
use transom_net::SystemSerialPortProvider;
use transom_store::{
    CatalogBackupService, JsonHistoryStore, JsonKnownHostsStore, JsonSessionStore,
    JsonSettingsStore, KeyringSecretStore, paths,
};

fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let urls = startup_urls();

    // 深链与单实例判定早于一切重量级初始化（钥匙串走 D-Bus 可能挂起秒级）：
    // 已有实例在跑时本进程只负责转交地址，不该建立存储、控制器与窗口。
    // `None` 即"地址已交接"，进程直接退出。
    #[cfg(unix)]
    let Some(primary) = deep_link::acquire_primary(&urls)? else {
        tracing::info!("地址已转交给正在运行的实例，本次启动结束");
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

    deliver_startup_urls(&controller, &urls);

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

/// 启动参数里的 `ssh://` 地址。
///
/// 非 Unix 平台返回空表：macOS/Windows 由平台层派发 URL 事件，这里若也接管
/// 会把同一条地址连两次。
fn startup_urls() -> Vec<String> {
    #[cfg(unix)]
    {
        deep_link::urls_from_args(std::env::args())
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// 把冷启动带来的地址投进事件总线。
///
/// 与运行期唤起共用同一条路径：界面事件泵启动后按到达顺序消费，不必为
/// "带地址启动"另开入口，也避免两个时机各写一遍连接逻辑。
fn deliver_startup_urls(controller: &Controller, urls: &[String]) {
    let event_tx = controller.event_sender();
    for url in urls {
        if let Err(error) = event_tx.try_send(AppEvent::DeepLink {
            url: Some(url.clone()),
        }) {
            tracing::error!("启动参数中的深链地址未能投递，已丢弃，原因: {error}");
        }
    }
}
