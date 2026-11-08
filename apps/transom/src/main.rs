mod controller;
mod deep_link;
mod logging;

use controller::Controller;
use std::sync::Arc;
use transom_core::{
    AppEvent, BackupService, DeepLinkRequest, HistoryStore, KnownHostsStore, LogLevel, SecretStore,
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
    let Some(primary) = deep_link::acquire_primary(&startup)? else {
        log::info!("唤起请求已转交给正在运行的实例，本次启动结束");
        return Ok(());
    };

    // 启动健康自检必须晚于单实例判定：被转交的第二个进程不是一次真实启动，
    // 若在这里累加崩溃计数，正常的多开唤起也会被误判成连续崩溃。
    // 自检只读写可执行文件旁的更新状态标记，失败只告警，不能把用户关在门外。
    match transom_updater::check_and_recover_on_startup() {
        Ok(transom_updater::RecoveryStatus::Normal) => {
            log::debug!("自更新启动自检通过，没有待确认的升级");
        }
        Ok(transom_updater::RecoveryStatus::PendingConfirmation { attempts }) => {
            log::info!("新版本处于健康观察期，当前为第 {attempts} 次启动尝试");
        }
        Ok(transom_updater::RecoveryStatus::RolledBack { from_version }) => {
            log::warn!("新版本 {from_version} 连续启动失败，已自动回滚到上一稳定版本");
        }
        Err(error) => {
            log::warn!("自更新启动自检失败，继续以当前版本启动: {error}");
        }
        _ => {
            log::debug!("自更新启动自检返回未知状态，按正常启动处理");
        }
    }

    let known_hosts: Arc<dyn KnownHostsStore> =
        Arc::new(JsonKnownHostsStore::new(paths::known_hosts_path())?);
    let session_store: Arc<dyn SessionStore> =
        Arc::new(JsonSessionStore::new(paths::sessions_path()));
    let settings_store: Arc<dyn SettingsStore> =
        Arc::new(JsonSettingsStore::new(paths::settings_path()));
    let secret_store: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new("transom"));
    let serial_ports: Arc<dyn SerialPortProvider> = Arc::new(SystemSerialPortProvider);
    let backup: Arc<dyn BackupService> = Arc::new(CatalogBackupService::new(secret_store.clone()));
    let history: Arc<dyn HistoryStore> = Arc::new(JsonHistoryStore::new(paths::history_path()));
    let git_projects: Arc<dyn transom_core::GitProjectStore> = Arc::new(
        transom_store::JsonGitProjectStore::new(paths::git_projects_path()),
    );

    let controller = Controller::start(known_hosts.clone(), secret_store.clone())?;

    // 监听句柄保留在此直到进程结束：随控制器停机，不做脱缰派生。
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
        git_projects,
        command_tx: controller.command_sender(),
        event_rx: controller.event_receiver(),
        event_tx: controller.event_sender(),
    };

    let restart_requested = transom_ui::run(services);
    // 优雅停机：控制平面断开全部远端、停止转发并限时收尾，随后关闭运行时。
    // 不能直接 drop：那会让收尾任务随运行时取消，远端收不到 DISCONNECT。
    controller.shutdown();
    // 更新安装完成后用户点了"立即重启"：此时旧进程的资源已经收尾，
    // 再拉起当前可执行文件的新进程并退出本进程。
    if restart_requested && let Err(error) = transom_updater::restart_current_process() {
        log::error!("更新已安装，但自动重启失败，请手动重新打开应用: {error}");
    }
    Ok(())
}

/// 启动参数解析出的唤起请求（`ssh://` 地址 + 可选目录）。
///
/// macOS 的 URL 由 LaunchServices 通过 `on_open_urls` 投递，不能再从 argv
/// 接管同一条地址；Linux/Windows 则从启动参数解析深链与目录/命令请求。
fn startup_request() -> StartupRequest {
    #[cfg(not(target_os = "macos"))]
    {
        deep_link::request_from_args(std::env::args())
    }
    #[cfg(target_os = "macos")]
    {
        deep_link::LaunchRequest::default()
    }
}

/// 跨平台统一的启动请求形态。
type StartupRequest = deep_link::LaunchRequest;

impl StartupRequest {
    /// 把冷启动请求按到达顺序展开成事件，与运行期唤起共用同一条投递路径：
    /// 界面事件泵启动后顺序消费，不必为"带参数启动"另开入口。
    fn into_events(self) -> Vec<AppEvent> {
        let mut events: Vec<AppEvent> = self
            .urls
            .into_iter()
            .map(|url| AppEvent::DeepLink {
                request: DeepLinkRequest::Ssh { url },
            })
            .collect();
        if let Some(command) = self.command {
            // 命令请求自带可选目录：整条请求只开一个标签，不能再单独发 Dir。
            events.push(AppEvent::DeepLink {
                request: DeepLinkRequest::Exec {
                    command,
                    dir: self.dir,
                },
            });
        } else if let Some(path) = self.dir {
            events.push(AppEvent::DeepLink {
                request: DeepLinkRequest::Dir { path },
            });
        }
        events
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
