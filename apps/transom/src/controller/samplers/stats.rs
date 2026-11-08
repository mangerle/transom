//! 系统指标采样器（CPU、内存、磁盘分段执行与降频调度）。

use super::common::send_overwritable;
use transom_core::FilesystemUsage;
use transom_core::{AppEvent, ConnectionId, HostAppEvent, TabId};
use transom_net::{SshRemote, StatsSamplerState};

/// 最近一次 df 结果的缓存形态：(已用字节, 总字节, 挂载点明细)。
pub(crate) type CachedDisk = (u64, u64, Vec<FilesystemUsage>);
pub(crate) type CachedDiag = (
    Vec<transom_core::PortListeningInfo>,
    Vec<transom_core::FailedServiceInfo>,
    Option<transom_core::OomAlert>,
);

/// 远端统计采样的周期。
pub(crate) const STATS_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// 磁盘明细（df）与系统诊断数据在统计采样中的降频比例：每 N 轮携带一次。
pub(crate) const DF_EVERY_N_ROUNDS: u64 = 3;
pub(crate) const DIAG_EVERY_N_ROUNDS: u64 = 3;

/// 单次采样命令与信号操作共用的超时。
pub(crate) const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 组装一轮统计采样的远端命令；OS 标识、df 与诊断段按需附带。
pub(crate) fn stats_sample_command(
    include_os: bool,
    include_df: bool,
    include_diag: bool,
) -> String {
    let mut cmd = String::from(
        "cat /proc/stat /proc/meminfo /proc/net/dev 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime /proc/loadavg 2>/dev/null; echo \"__DF__\"",
    );
    if include_df {
        cmd.push_str("; df -P -k 2>/dev/null");
    }
    cmd.push_str("; echo \"__NET__\"; cat /proc/net/dev 2>/dev/null");
    if include_os {
        cmd.push_str("; echo \"__OS__\"; cat /etc/os-release 2>/dev/null");
    }
    if include_diag {
        cmd.push_str("; echo \"__PORTS__\"; (ss -tulpn 2>/dev/null || netstat -tulpn 2>/dev/null); echo \"__SERVICES__\"; systemctl list-units --type=service --state=failed --no-legend 2>/dev/null; echo \"__OOM__\"; (dmesg -T 2>/dev/null || dmesg 2>/dev/null) | grep -i \"oom-killer\" | tail -n 1");
    }
    cmd
}

/// 远端系统统计采样：每 [`STATS_SAMPLE_INTERVAL`] exec 一组只读命令。
pub(crate) fn spawn_remote_stats_sampler(
    remote: SshRemote,
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
    sampling: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut state = StatsSamplerState::default();
        let mut interval = tokio::time::interval(STATS_SAMPLE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        interval.tick().await;

        let mut cached_os: Option<String> = None;
        let mut cached_disk: Option<CachedDisk> = None;
        let mut cached_diag: Option<CachedDiag> = None;
        let mut timeout_warned = false;
        let mut round: u64 = 0;
        loop {
            if !sampling.load(std::sync::atomic::Ordering::Relaxed) {
                interval.tick().await;
                continue;
            }
            let include_df = round.is_multiple_of(DF_EVERY_N_ROUNDS);
            let include_diag = round.is_multiple_of(DIAG_EVERY_N_ROUNDS);
            round = round.wrapping_add(1);
            let cmd = stats_sample_command(cached_os.is_none(), include_df, include_diag);
            let exec = tokio::time::timeout(COMMAND_TIMEOUT, remote.exec_command(&cmd)).await;
            match exec {
                Ok(Ok(output)) if !output.is_empty() => {
                    let mut stats = state.ingest(&output, std::time::Instant::now());
                    match &cached_os {
                        Some(cached) => stats.os_name = cached.clone(),
                        None => cached_os = Some(stats.os_name.clone()),
                    }
                    if include_df {
                        cached_disk = Some((
                            stats.disk_used_bytes,
                            stats.disk_total_bytes,
                            stats.filesystems.clone(),
                        ));
                    } else if let Some((used, total, filesystems)) = &cached_disk {
                        stats.disk_used_bytes = *used;
                        stats.disk_total_bytes = *total;
                        stats.filesystems.clone_from(filesystems);
                    }
                    if include_diag {
                        cached_diag = Some((
                            stats.ports.clone(),
                            stats.failed_services.clone(),
                            stats.recent_oom.clone(),
                        ));
                    } else if let Some((ports, failed_services, recent_oom)) = &cached_diag {
                        stats.ports.clone_from(ports);
                        stats.failed_services.clone_from(failed_services);
                        stats.recent_oom.clone_from(recent_oom);
                    }
                    if !send_overwritable(
                        &event_tx,
                        HostAppEvent::SystemStatsUpdated {
                            tab_id,
                            connection_id,
                            stats: Box::new(stats),
                        }
                        .into(),
                        "系统统计",
                    ) {
                        break;
                    }
                }
                // 空输出（远端命令整体失败）或通道错误：连接已不可用，停止采样。
                Ok(_) => break,
                // 超时：连接可能仍然健康，只跳过本轮并保留上一份数据；首次按
                // warn 留痕，持续超时降为 debug，避免每轮刷一条告警。
                Err(_) => {
                    if timeout_warned {
                        log::debug!("远端系统统计采样再次超时，已跳过本轮，标签: {tab_id}");
                    } else {
                        timeout_warned = true;
                        log::warn!(
                            "远端系统统计采样超时，已跳过本轮并保留上一份数据，标签: {tab_id}，超时: {} 秒",
                            COMMAND_TIMEOUT.as_secs()
                        );
                    }
                }
            }
            interval.tick().await;
        }
    })
}
