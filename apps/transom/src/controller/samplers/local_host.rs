//! 本地宿主的系统指标与网络流量共享采样。
//!
//! 设计原理（Why）：两者都是整台机器的属性，全部本地终端应共享同一份
//! 采样器；系统快照每三秒更新，网络速率每秒更新。`sysinfo` 刷新属于阻塞
//! 系统调用，统一放进 `spawn_blocking`，不占用异步 worker。
//!
//! 设计原理（Why）：快照使用平台层结构化数据，不再读取 `/proc` 或启动
//! `sh`、`df`、`ss`、`systemctl`，因此 Linux、macOS、Windows 共用一条路径。

use super::common::send_overwritable;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use transom_core::{
    AppEvent, ConnectionId, CoreUsage, FilesystemUsage, HostAppEvent, InterfaceRate,
    NetworkTraffic, SystemStats, SystemStatsCapabilities, TabId,
};
use transom_platform::system::{HostSampler, NetworkTotals, SystemSnapshot};

/// 本地宿主采样订阅表。
pub(crate) type LocalHostTabs = Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const SYSTEM_EVERY_N_ROUNDS: u64 = 3;

/// 运行本地宿主共享采样器；订阅清空或事件通道关闭后退出。
pub(crate) async fn run_local_host_sampler(
    event_tx: flume::Sender<AppEvent>,
    tabs: LocalHostTabs,
    sampling: Arc<AtomicBool>,
) {
    let sampler = Arc::new(parking_lot::Mutex::new(HostSampler::new()));
    let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;

    let mut previous: Option<NetworkTotals> = None;
    let mut previous_at = Instant::now();
    let mut round = 0u64;
    loop {
        interval.tick().await;
        let subscribers = match tabs.lock() {
            Ok(tabs) if tabs.is_empty() => break,
            Ok(tabs) => tabs.clone(),
            Err(_) => break,
        };
        if previous.is_none()
            && !broadcast_traffic(&event_tx, &subscribers, NetworkTraffic::default())
        {
            break;
        }

        let sampler_for_network = Arc::clone(&sampler);
        let network =
            tokio::task::spawn_blocking(move || sampler_for_network.lock().sample_network()).await;
        let network = match network {
            Ok(network) => network,
            Err(error) => {
                log::error!("本地网络采样任务异常终止: {error}");
                break;
            }
        };
        let now = Instant::now();
        if let Some(last) = previous {
            let traffic = NetworkTraffic::from_delta(
                (last.received_bytes, last.transmitted_bytes),
                (network.received_bytes, network.transmitted_bytes),
                now.saturating_duration_since(previous_at),
            );
            if !broadcast_traffic(&event_tx, &subscribers, traffic) {
                break;
            }
        }
        previous = Some(network);
        previous_at = now;

        if sampling.load(Ordering::Relaxed) && round.is_multiple_of(SYSTEM_EVERY_N_ROUNDS) {
            let sampler_for_system = Arc::clone(&sampler);
            match tokio::task::spawn_blocking(move || sampler_for_system.lock().sample_system())
                .await
            {
                Ok(snapshot) => {
                    if !broadcast_stats(&event_tx, &subscribers, snapshot) {
                        break;
                    }
                }
                Err(error) => {
                    log::error!("本地系统统计采样任务异常终止: {error}");
                    break;
                }
            }
        }
        round = round.wrapping_add(1);
    }
}

fn broadcast_traffic(
    event_tx: &flume::Sender<AppEvent>,
    subscribers: &[(TabId, ConnectionId)],
    traffic: NetworkTraffic,
) -> bool {
    for (tab_id, connection_id) in subscribers {
        if !send_overwritable(
            event_tx,
            HostAppEvent::NetworkTrafficUpdated {
                tab_id: *tab_id,
                connection_id: *connection_id,
                traffic,
            }
            .into(),
            "本地网络流量",
        ) {
            return false;
        }
    }
    true
}

fn broadcast_stats(
    event_tx: &flume::Sender<AppEvent>,
    subscribers: &[(TabId, ConnectionId)],
    snapshot: SystemSnapshot,
) -> bool {
    let stats = Box::new(map_system_snapshot(snapshot));
    for (tab_id, connection_id) in subscribers {
        if !send_overwritable(
            event_tx,
            HostAppEvent::SystemStatsUpdated {
                tab_id: *tab_id,
                connection_id: *connection_id,
                stats: stats.clone(),
            }
            .into(),
            "本地系统统计",
        ) {
            return false;
        }
    }
    true
}

fn map_system_snapshot(snapshot: SystemSnapshot) -> SystemStats {
    SystemStats {
        cpu_percent: snapshot.cpu_percent,
        mem_used_bytes: snapshot.mem_used_bytes,
        mem_total_bytes: snapshot.mem_total_bytes,
        disk_used_bytes: snapshot.disk_used_bytes,
        disk_total_bytes: snapshot.disk_total_bytes,
        uptime_secs: snapshot.uptime_secs,
        os_name: snapshot.os_name,
        load_avg: snapshot.load_avg,
        cpu_iowait_percent: 0.0,
        cpu_system_percent: 0.0,
        cpu_user_percent: 0.0,
        swap_used_bytes: snapshot.swap_used_bytes,
        swap_total_bytes: snapshot.swap_total_bytes,
        cpu_cores: snapshot
            .cpu_cores
            .into_iter()
            .map(|core| CoreUsage {
                index: core.index,
                percent: core.percent,
            })
            .collect(),
        filesystems: snapshot
            .filesystems
            .into_iter()
            .map(|filesystem| FilesystemUsage {
                mount: filesystem.mount,
                used_bytes: filesystem.used_bytes,
                total_bytes: filesystem.total_bytes,
            })
            .collect(),
        interfaces: snapshot
            .interfaces
            .into_iter()
            .map(|interface| InterfaceRate {
                name: interface.name,
                rx_bytes_per_sec: interface.rx_bytes_per_sec,
                tx_bytes_per_sec: interface.tx_bytes_per_sec,
            })
            .collect(),
        ports: Vec::new(),
        failed_services: Vec::new(),
        recent_oom: None,
        capabilities: SystemStatsCapabilities::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::map_system_snapshot;
    use transom_platform::system::{CoreUsage, SystemSnapshot};

    #[test]
    fn mapping_marks_linux_only_capabilities_unavailable() {
        let stats = map_system_snapshot(SystemSnapshot {
            cpu_cores: vec![CoreUsage {
                index: 0,
                percent: 42.0,
            }],
            ..Default::default()
        });
        assert_eq!(stats.cpu_cores.len(), 1);
        assert!(!stats.capabilities.cpu_breakdown);
        assert!(!stats.capabilities.linux_diagnostics);
    }
}
