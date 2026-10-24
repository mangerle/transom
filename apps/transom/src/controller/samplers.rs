//! 状态栏采样器：网络速率与系统统计（远端/本地共享两套）。

use std::sync::Arc;
use tokio::process::Command;
use transom_core::traffic::parse_net_dev_totals;
use transom_core::{AppEvent, ConnectionId, NetworkTraffic, TabId, TrafficCounter};
use transom_net::SshRemote;

pub(super) fn spawn_traffic_sampler(
    traffic: Arc<TrafficCounter>,
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // 先交一个零样本，让网络徽标在连接建立后立刻出现；首个真实速率
        // 仍从完整的一秒窗口开始计算，避免瞬时大数字。
        if event_tx
            .send_async(AppEvent::NetworkTrafficUpdated {
                tab_id,
                connection_id,
                traffic: NetworkTraffic::default(),
            })
            .await
            .is_err()
        {
            return;
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // 跳过 interval 的第一个瞬时 tick，保证首个速率窗口是完整的 1 秒。
        interval.tick().await;
        let mut previous = traffic.totals();
        let mut previous_at = std::time::Instant::now();
        loop {
            interval.tick().await;
            let now = std::time::Instant::now();
            let elapsed = now.saturating_duration_since(previous_at);
            previous_at = now;
            let current = traffic.totals();
            let sample = NetworkTraffic::from_delta(previous, current, elapsed);
            previous = current;
            if event_tx
                .send_async(AppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: sample,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

/// 本地终端的网络采样：读取本机 `/proc/net/dev` 的非回环网卡累计字节。
///
/// 与 SSH 传输层计数不同，这里得到的是整台机器所有非回环接口的合计吞吐，
/// 因此界面上要按会话类型区分提示文案。读取失败时跳过本轮并保留上一次
/// 有效样本，避免把一次文件读取抖动显示成断网。
pub(super) fn spawn_local_traffic_sampler(
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    async fn read_totals() -> Option<(u64, u64)> {
        let content = tokio::fs::read_to_string("/proc/net/dev").await.ok()?;
        parse_net_dev_totals(&content)
    }

    tokio::spawn(async move {
        if event_tx
            .send_async(AppEvent::NetworkTrafficUpdated {
                tab_id,
                connection_id,
                traffic: NetworkTraffic::default(),
            })
            .await
            .is_err()
        {
            return;
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        let mut previous = read_totals().await;
        let mut previous_at = std::time::Instant::now();
        loop {
            interval.tick().await;
            let Some(current) = read_totals().await else {
                continue;
            };
            let now = std::time::Instant::now();
            let elapsed = now.saturating_duration_since(previous_at);
            previous_at = now;
            let Some(previous_totals) = previous.replace(current) else {
                continue;
            };
            let sample = NetworkTraffic::from_delta(previous_totals, current, elapsed);
            if event_tx
                .send_async(AppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: sample,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

/// 远端系统统计采样：每 3 秒 exec 一组只读命令，OS 标识首轮后缓存复用。
/// 返回 JoinHandle 由调用方纳入 stats_tasks 跟踪，断开时中止。
pub(super) fn spawn_remote_stats_sampler(
    remote: SshRemote,
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev_cpu = None;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // 跳过 interval 的第一个瞬时 tick，保证首个采样窗口是完整的
        // 3 秒；否则首个 tick 立即完成，前两次采样背靠背，CPU 占比失真。
        interval.tick().await;
        // OS 标识是静态信息：仅首轮附带 os-release，之后缓存复用，
        // 省掉每 3 秒一次重复读取静态文件的开销。
        let mut cached_os: Option<String> = None;
        loop {
            let cmd = if cached_os.is_none() {
                "cat /proc/stat /proc/meminfo 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k / 2>/dev/null; echo \"__OS__\"; cat /etc/os-release 2>/dev/null"
            } else {
                "cat /proc/stat /proc/meminfo 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k / 2>/dev/null"
            };
            match remote.exec_command(cmd).await {
                Ok(output) if !output.is_empty() => {
                    let (mut stats, curr_cpu) =
                        transom_core::system_stats::parse_combined_stats(&output, prev_cpu);
                    prev_cpu = curr_cpu;
                    match &cached_os {
                        Some(cached) => stats.os_name = cached.clone(),
                        None => cached_os = Some(stats.os_name.clone()),
                    }
                    if event_tx
                        .send_async(AppEvent::SystemStatsUpdated {
                            tab_id,
                            connection_id,
                            stats,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                _ => break,
            }
            interval.tick().await;
        }
    })
}

/// 本地终端共享的系统统计采样器。
///
/// 设计原理（Why）：
/// - 实现初衷：统计口径是"整台机器"（CPU/内存/磁盘），多个本地标签
///   各自采集只会得到 N 份相同数据，却付出 N 倍 /proc 读取与 df 子进程；
/// - 核心优势：采集按机器计一次，结果广播给全部订阅标签；OS 标识是
///   静态信息，首轮读取后缓存，后续轮次不再碰 /etc/os-release；
/// - 代价与局限：采样周期内新登记的标签要等下一轮才收到数据（3 秒），
///   对状态栏指标无感知影响。
pub(super) async fn run_local_stats_sampler(
    event_tx: flume::Sender<AppEvent>,
    local_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
) {
    let mut prev_cpu = None;
    let mut cached_os: Option<String> = None;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // 跳过首个瞬时 tick，保证首个采样窗口是完整的 3 秒。
    interval.tick().await;
    loop {
        // 持锁只做纯内存克隆，不跨 await；订阅清空即退出，
        // 下一个本地连接会重新拉起本采样器。
        let tabs = match local_tabs.lock() {
            Ok(tabs) if tabs.is_empty() => break,
            Ok(tabs) => tabs.clone(),
            Err(_) => break,
        };
        let stat = tokio::fs::read_to_string("/proc/stat")
            .await
            .unwrap_or_default();
        let mem = tokio::fs::read_to_string("/proc/meminfo")
            .await
            .unwrap_or_default();
        let uptime = tokio::fs::read_to_string("/proc/uptime")
            .await
            .unwrap_or_default();
        let os = match &cached_os {
            Some(_) => String::new(),
            None => tokio::fs::read_to_string("/etc/os-release")
                .await
                .unwrap_or_default(),
        };
        let df = match Command::new("df").args(["-P", "-k", "/"]).output().await {
            Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
            Err(_) => String::new(),
        };
        let output = format!("{stat}\n{mem}\n__UPTIME__\n{uptime}\n__DF__\n{df}\n__OS__\n{os}");
        let (mut stats, curr_cpu) =
            transom_core::system_stats::parse_combined_stats(&output, prev_cpu);
        prev_cpu = curr_cpu;
        match &cached_os {
            Some(cached) => stats.os_name = cached.clone(),
            None => cached_os = Some(stats.os_name.clone()),
        }
        for (tab_id, connection_id) in tabs {
            if event_tx
                .send_async(AppEvent::SystemStatsUpdated {
                    tab_id,
                    connection_id,
                    stats: stats.clone(),
                })
                .await
                .is_err()
            {
                return;
            }
        }
        interval.tick().await;
    }
}
