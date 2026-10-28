//! 系统指标采样器（CPU、内存、磁盘分段执行与降频调度）。

use super::common::send_overwritable;
use std::sync::Arc;
use tokio::process::Command;
use transom_core::system_stats::FilesystemUsage;
use transom_core::{AppEvent, ConnectionId, HostAppEvent, TabId};
use transom_net::{SshRemote, StatsSamplerState};

/// 最近一次 df 结果的缓存形态：(已用字节, 总字节, 挂载点明细)。
pub(crate) type CachedDisk = (u64, u64, Vec<FilesystemUsage>);

/// 远端统计采样的周期。
///
/// 每轮在远端执行一组 /proc 读取（部分轮次附带 df 子进程），对低配远端
/// 是可见负载；5 秒的刷新率对状态栏与看板足够，较 3 秒口径远端负载下降
/// 约四成。
pub(crate) const STATS_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// 磁盘明细（df）在统计采样中的降频比例：每 N 轮携带一次 df。
///
/// 为什么单独降频：df 是子进程且会在挂死挂载点上阻塞（虽有超时兜底），
/// 而磁盘占用是慢变量；其余轮次复用上一次结果，把子进程开销摊薄到 1/N。
/// 本地采样器共用同一比例。
pub(crate) const DF_EVERY_N_ROUNDS: u64 = 3;

/// 单次采样命令与信号操作共用的超时（远端 exec 与本地子进程同口径）：
/// 远端 `ps`/`df` 或本地命令卡死不能拖住后续轮询与统计采样。
pub(crate) const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 组装一轮统计采样的远端命令；OS 标识与 df 段按需附带。
///
/// `__DF__` 与 `__NET__` 分隔标记必须始终保留：解析器在缺标记时会回退到
/// 全文扫描（兼容旧格式），省略标记会把 /proc 内容误当 df 行解析。
pub(crate) fn stats_sample_command(include_os: bool, include_df: bool) -> String {
    let mut cmd = String::from(
        "cat /proc/stat /proc/meminfo /proc/net/dev 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"",
    );
    if include_df {
        cmd.push_str("; df -P -k 2>/dev/null");
    }
    cmd.push_str("; echo \"__NET__\"; cat /proc/net/dev 2>/dev/null");
    if include_os {
        cmd.push_str("; echo \"__OS__\"; cat /etc/os-release 2>/dev/null");
    }
    cmd
}

/// 远端系统统计采样：每 [`STATS_SAMPLE_INTERVAL`] exec 一组只读命令，
/// OS 标识首轮后缓存复用，df 每 [`DF_EVERY_N_ROUNDS`] 轮执行一次（其余
/// 轮次复用上一次的磁盘结果）。返回 JoinHandle 由调用方纳入 stats_tasks
/// 跟踪，断开时中止。
pub(crate) fn spawn_remote_stats_sampler(
    remote: SshRemote,
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut state = StatsSamplerState::default();
        let mut interval = tokio::time::interval(STATS_SAMPLE_INTERVAL);
        // exec 耗时超过周期时不得连发补采：丢弃错过的 tick，保持完整采样窗口。
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // 跳过 interval 的第一个瞬时 tick，保证首个采样窗口是完整的
        // 5 秒；否则首个 tick 立即完成，前两次采样背靠背，CPU 占比失真。
        interval.tick().await;
        // OS 标识是静态信息：仅首轮附带 os-release，之后缓存复用，
        // 省掉每轮一次重复读取静态文件的开销。
        let mut cached_os: Option<String> = None;
        // 最近一次 df 结果：无 df 的轮次用它回填，磁盘指标保持连续。
        // 首轮必然带 df（round 从 0 起），因此缓存一定先被填上。
        let mut cached_disk: Option<CachedDisk> = None;
        // 超时告警是否已留痕：持续超时时只在首次打 warn，避免刷屏。
        let mut timeout_warned = false;
        let mut round: u64 = 0;
        loop {
            let include_df = round.is_multiple_of(DF_EVERY_N_ROUNDS);
            round = round.wrapping_add(1);
            let cmd = stats_sample_command(cached_os.is_none(), include_df);
            // 远端 exec 必须设限：df 撞上挂死的 NFS 挂载会长时间不返回，
            // 采样任务随之挂起，直到 keepalive 断开前该标签的统计一直停更。
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
                        // 本轮没执行 df：复用上一次的磁盘结果，避免界面
                        // 上磁盘指标跳 0。
                        stats.disk_used_bytes = *used;
                        stats.disk_total_bytes = *total;
                        stats.filesystems.clone_from(filesystems);
                    }
                    if !send_overwritable(
                        &event_tx,
                        HostAppEvent::SystemStatsUpdated {
                            tab_id,
                            connection_id,
                            stats,
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

/// 本地终端共享的系统统计采样器。
///
/// 设计原理（Why）：
/// - 实现初衷：统计口径是"整台机器"（CPU/内存/磁盘），多个本地标签
///   各自采集只会得到 N 份相同数据，却付出 N 倍 /proc 读取与 df 子进程；
/// - 核心优势：采集按机器计一次，结果广播给全部订阅标签；OS 标识是
///   静态信息，首轮读取后缓存，后续轮次不再碰 /etc/os-release；
/// - 代价与局限：采样周期内新登记的标签要等下一轮才收到数据（3 秒），
///   对状态栏指标无感知影响。
pub(crate) async fn run_local_stats_sampler(
    event_tx: flume::Sender<AppEvent>,
    local_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
) {
    let mut state = StatsSamplerState::default();
    let mut cached_os: Option<String> = None;
    // 与远端采样同口径：df 每 DF_EVERY_N_ROUNDS 轮执行一次，其余轮次
    // 复用上一次的磁盘结果，把子进程开销摊薄到 1/N。
    let mut cached_disk: Option<CachedDisk> = None;
    let mut round: u64 = 0;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    // 采样耗时超过周期时不得连发补采：丢弃错过的 tick，保持完整采样窗口。
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
        let net = tokio::fs::read_to_string("/proc/net/dev")
            .await
            .unwrap_or_default();
        let os = match &cached_os {
            Some(_) => String::new(),
            None => tokio::fs::read_to_string("/etc/os-release")
                .await
                .unwrap_or_default(),
        };
        let include_df = round.is_multiple_of(DF_EVERY_N_ROUNDS);
        round = round.wrapping_add(1);
        // df 是外部子进程：必须限时并保证退出收割，防止异常环境下
        // 子进程假死拖住整轮采样；无 df 的轮次只保留空段标记（解析器
        // 缺标记会回退全文扫描，标记不可省略），结果由缓存回填。
        let df = if include_df {
            let mut df_command = Command::new("df");
            df_command.args(["-P", "-k"]).kill_on_drop(true);
            match tokio::time::timeout(std::time::Duration::from_secs(5), df_command.output()).await
            {
                Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).to_string(),
                Ok(Err(_)) | Err(_) => String::new(),
            }
        } else {
            String::new()
        };
        let output = format!(
            "{stat}\n{mem}\n__UPTIME__\n{uptime}\n__DF__\n{df}\n__NET__\n{net}\n__OS__\n{os}"
        );
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
        for (tab_id, connection_id) in tabs {
            if !send_overwritable(
                &event_tx,
                HostAppEvent::SystemStatsUpdated {
                    tab_id,
                    connection_id,
                    stats: stats.clone(),
                }
                .into(),
                "系统统计",
            ) {
                return;
            }
        }
        interval.tick().await;
    }
}
