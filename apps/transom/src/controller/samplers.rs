//! 状态栏采样器：网络速率与系统统计（远端/本地共享两套）。

use std::sync::Arc;
use tokio::process::Command;
use transom_core::system_stats::FilesystemUsage;
use transom_core::traffic::parse_net_dev_totals;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, NetworkTraffic, StatsSamplerState, TabId,
    TrafficCounter,
};
use transom_net::SshRemote;

/// 最近一次 df 结果的缓存形态：(已用字节, 总字节, 挂载点明细)。
type CachedDisk = (u64, u64, Vec<FilesystemUsage>);

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

/// 本地终端的网络采样订阅表：与共享系统统计同一模式，按机器计一份。
pub(super) type LocalTrafficTabs = Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>;

/// 本地终端的网络采样（机器级共享）：读取本机 `/proc/net/dev` 的非回环
/// 网卡累计字节，广播给全部已登记标签。
///
/// 设计原理（Why）：本地网络流量是整台机器的属性（与 SSH 传输层计数不同），
/// 多个本地标签各自读取同一份 /proc/net/dev 只会得到 N 份相同数据；共享
/// 采样按机器计一次读取，结果逐标签广播。
/// 界面上按会话类型区分提示文案由渲染层负责。读取失败时本轮跳过并保留
/// 上一次有效样本，避免把一次文件读取抖动显示成断网。
pub(super) async fn run_local_traffic_sampler(
    event_tx: flume::Sender<AppEvent>,
    tabs: LocalTrafficTabs,
) {
    async fn read_totals() -> Option<(u64, u64)> {
        let content = tokio::fs::read_to_string("/proc/net/dev").await.ok()?;
        parse_net_dev_totals(&content)
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // 与其他采样器同口径：跳过首个瞬时 tick，保证首个速率窗口完整 1 秒。
    interval.tick().await;
    let mut previous: Option<(u64, u64)> = read_totals().await;
    let mut previous_at = std::time::Instant::now();
    loop {
        // 持锁只做纯内存克隆，不跨 await；订阅清空即退出，
        // 下一个本地连接会重新拉起本采样器。
        let subscribers = match tabs.lock() {
            Ok(tabs) if tabs.is_empty() => break,
            Ok(tabs) => tabs.clone(),
            Err(_) => break,
        };
        let Some(current) = read_totals().await else {
            // 读取失败本轮跳过并保留上一次有效样本；必须等待下一个采样窗口
            // 再继续——/proc/net/dev 不可读（受限容器、隐藏 /proc）时会持续
            // 失败，跳过 tick 立即重试会退化成无间隔热循环，白白占满一个
            // 异步工作线程。
            interval.tick().await;
            continue;
        };
        let now = std::time::Instant::now();
        let elapsed = now.saturating_duration_since(previous_at);
        previous_at = now;
        let Some(previous_totals) = previous.replace(current) else {
            // 首轮没有可比较的基准样本：同样等待下一个采样窗口再计算速率，
            // 与其它分支保持同一节奏，不做无间隔重试。
            interval.tick().await;
            continue;
        };
        let sample = NetworkTraffic::from_delta(previous_totals, current, elapsed);
        let mut channel_closed = false;
        for (tab_id, connection_id) in subscribers {
            if event_tx
                .send_async(AppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: sample,
                })
                .await
                .is_err()
            {
                channel_closed = true;
                break;
            }
        }
        if channel_closed {
            break;
        }
    }
}

/// 远端统计采样的周期。
///
/// 每轮在远端执行一组 /proc 读取（部分轮次附带 df 子进程），对低配远端
/// 是可见负载；5 秒的刷新率对状态栏与看板足够，较 3 秒口径远端负载下降
/// 约四成。
const STATS_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// 磁盘明细（df）在统计采样中的降频比例：每 N 轮携带一次 df。
///
/// 为什么单独降频：df 是子进程且会在挂死挂载点上阻塞（虽有超时兜底），
/// 而磁盘占用是慢变量；其余轮次复用上一次结果，把子进程开销摊薄到 1/N。
/// 本地采样器共用同一比例。
const DF_EVERY_N_ROUNDS: u64 = 3;

/// 组装一轮统计采样的远端命令；OS 标识与 df 段按需附带。
///
/// `__DF__` 与 `__NET__` 分隔标记必须始终保留：解析器在缺标记时会回退到
/// 全文扫描（兼容旧格式），省略标记会把 /proc 内容误当 df 行解析。
fn stats_sample_command(include_os: bool, include_df: bool) -> String {
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
pub(super) fn spawn_remote_stats_sampler(
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
pub(super) async fn run_local_stats_sampler(
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

/// 进程采样/信号控制的目标：SSH 会话走远端 exec 通道，本地终端跑本机命令。
///
/// 串口与 Telnet 不参与进程面板：控制器只在远端已登记或调用方确认本地终端时
/// 才会收到命令，按 `remote` 是否为 `Some` 选择执行位置。
pub(super) struct ProcessTarget {
    pub(super) remote: Option<SshRemote>,
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) event_tx: flume::Sender<AppEvent>,
}

/// 按 CPU 降序取前 50 个进程的一行采样命令。
///
/// 与 `ps` 默认输出不同，`--sort=-%cpu` 让最活跃的进程一定在面板顶部；
/// `head -n 51` 保留表头在内的 51 行，远端输出量可控。
const PROCESS_SAMPLE_COMMAND: &str =
    "ps -eo pid,ppid,user,%cpu,%mem,rss,comm --sort=-%cpu | head -n 51";

/// 单次采样命令与信号操作共用的超时（远端 exec 与本地子进程同口径）：
/// 远端 `ps`/`df` 或本地命令卡死不能拖住后续轮询与统计采样。
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(super) async fn run_process_sample(target: ProcessTarget) {
    let result = match &target.remote {
        Some(remote) => {
            match tokio::time::timeout(
                COMMAND_TIMEOUT,
                remote.exec_command_capture(PROCESS_SAMPLE_COMMAND),
            )
            .await
            {
                Ok(Ok(output)) => Ok(output.stdout),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(command_timeout("进程采样")),
            }
        }
        None => local_command_output(PROCESS_SAMPLE_COMMAND).await,
    };
    let event = match result {
        Ok(output) => {
            let processes = transom_core::system_stats::parse_processes(&output);
            AppEvent::ProcessesSampled {
                tab_id: target.tab_id,
                connection_id: target.connection_id,
                processes,
            }
        }
        Err(error) => AppEvent::ProcessesSampleFailed {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            error,
        },
    };
    let _ = target.event_tx.send_async(event).await;
}

pub(super) async fn run_process_kill(target: ProcessTarget, pid: u32, force: bool) {
    // PID 1 是 init/systemd：误杀会让整台远端主机失去服务管理能力，
    // 面板上即便手工伪造请求也必须在执行层拦下。
    let error = if pid <= 1 {
        Some(AppError::with_detail(
            ErrorCode::Configuration,
            "拒绝向 PID 1（init/systemd）发送信号".to_string(),
        ))
    } else {
        let signal = if force { "-9" } else { "-15" };
        match &target.remote {
            Some(remote) => {
                let command = format!("kill {signal} {pid}");
                match tokio::time::timeout(COMMAND_TIMEOUT, remote.exec_command_capture(&command))
                    .await
                {
                    Ok(Ok(output)) if output.exit_status == Some(0) => None,
                    Ok(Ok(output)) => Some(AppError::with_detail(
                        ErrorCode::ForwardFailed,
                        format!("远端拒绝终止进程 {pid}，错误输出: {}", output.stderr.trim()),
                    )),
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some(command_timeout("终止进程")),
                }
            }
            None => match local_kill(pid, signal).await {
                Ok(()) => None,
                Err(reason) => Some(AppError::with_detail(ErrorCode::Io, reason)),
            },
        }
    };
    let _ = target
        .event_tx
        .send_async(AppEvent::ProcessKilled {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            pid,
            error,
        })
        .await;
}

fn command_timeout(action: &str) -> AppError {
    AppError::with_detail(
        ErrorCode::Timeout,
        format!("{action}超时，已等待 {} 秒", COMMAND_TIMEOUT.as_secs()),
    )
}

async fn local_command_output(command: &str) -> Result<String, AppError> {
    let mut child = Command::new("sh");
    child.arg("-c").arg(command).kill_on_drop(true);
    match tokio::time::timeout(COMMAND_TIMEOUT, child.output()).await {
        Ok(Ok(output)) => Ok(String::from_utf8_lossy(&output.stdout).to_string()),
        Ok(Err(error)) => Err(AppError::with_detail(
            ErrorCode::Io,
            format!("执行本地进程采样失败: {error}"),
        )),
        Err(_) => Err(command_timeout("本地进程采样")),
    }
}

async fn local_kill(pid: u32, signal: &str) -> Result<(), String> {
    let mut child = Command::new("kill");
    child.arg(signal).arg(pid.to_string()).kill_on_drop(true);
    let status = tokio::time::timeout(COMMAND_TIMEOUT, child.status())
        .await
        .map_err(|_| "本地 kill 命令超时".to_string())?
        .map_err(|error| format!("执行本地 kill 失败: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("本地 kill 返回失败状态: {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{DF_EVERY_N_ROUNDS, stats_sample_command};

    /// 命令必须始终保留段落标记：解析器在缺标记时会回退全文扫描，
    /// 省略 `__DF__`/`__NET__` 会把 /proc 内容误当 df 行解析。
    #[test]
    fn stats_sample_command_keeps_section_markers() {
        let lean = stats_sample_command(false, false);
        assert!(lean.contains("__UPTIME__"), "运行时长标记不可省略");
        assert!(lean.contains("__DF__"), "df 标记不可省略");
        assert!(lean.contains("__NET__"), "网卡标记不可省略");
        assert!(!lean.contains("__OS__"), "非首轮不应附带 os-release");
        assert!(!lean.contains("df -P -k"), "无 df 轮次不得启动 df 子进程");
    }

    /// 首轮与 df 轮次的可选段：OS 段与 df 命令按需出现。
    #[test]
    fn stats_sample_command_includes_optional_sections() {
        let full = stats_sample_command(true, true);
        assert!(full.contains("__OS__"), "首轮必须附带 os-release");
        assert!(full.contains("df -P -k"), "df 轮次必须执行 df 子进程");
    }

    /// 降频比例是 1/N 语义：第 0 轮带 df，随后 N-1 轮不带，周期恰好为 N。
    #[test]
    fn df_scheduling_cycles_every_n_rounds() {
        let with_df: Vec<u64> = (0..DF_EVERY_N_ROUNDS * 2)
            .filter(|round| round.is_multiple_of(DF_EVERY_N_ROUNDS))
            .collect();
        assert_eq!(
            with_df,
            vec![0, DF_EVERY_N_ROUNDS],
            "df 轮次应按固定周期出现"
        );
    }
}
