//! 状态栏采样器：网络速率与系统统计（远端/本地共享两套）。

use std::sync::Arc;
use tokio::process::Command;
use transom_core::traffic::parse_net_dev_totals;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, NetworkTraffic, StatsSamplerState, TabId,
    TrafficCounter,
};
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

/// 远端系统统计采样：每 3 秒 exec 一组只读命令，OS 标识首轮后缓存复用。
/// 返回 JoinHandle 由调用方纳入 stats_tasks 跟踪，断开时中止。
pub(super) fn spawn_remote_stats_sampler(
    remote: SshRemote,
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut state = StatsSamplerState::default();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
        // exec 耗时超过周期时不得连发补采：丢弃错过的 tick，保持完整采样窗口。
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // 跳过 interval 的第一个瞬时 tick，保证首个采样窗口是完整的
        // 3 秒；否则首个 tick 立即完成，前两次采样背靠背，CPU 占比失真。
        interval.tick().await;
        // OS 标识是静态信息：仅首轮附带 os-release，之后缓存复用，
        // 省掉每 3 秒一次重复读取静态文件的开销。
        let mut cached_os: Option<String> = None;
        loop {
            let cmd = if cached_os.is_none() {
                "cat /proc/stat /proc/meminfo /proc/net/dev 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k 2>/dev/null; echo \"__NET__\"; cat /proc/net/dev 2>/dev/null; echo \"__OS__\"; cat /etc/os-release 2>/dev/null"
            } else {
                "cat /proc/stat /proc/meminfo /proc/net/dev 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k 2>/dev/null; echo \"__NET__\"; cat /proc/net/dev 2>/dev/null"
            };
            match remote.exec_command(cmd).await {
                Ok(output) if !output.is_empty() => {
                    let mut stats = state.ingest(&output, std::time::Instant::now());
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
    let mut state = StatsSamplerState::default();
    let mut cached_os: Option<String> = None;
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
        // df 是外部子进程：必须限时并保证退出收割，防止异常环境下
        // 子进程假死拖住整轮采样
        let mut df_command = Command::new("df");
        df_command.args(["-P", "-k"]).kill_on_drop(true);
        let df = match tokio::time::timeout(std::time::Duration::from_secs(5), df_command.output())
            .await
        {
            Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).to_string(),
            Ok(Err(_)) | Err(_) => String::new(),
        };
        let output = format!(
            "{stat}\n{mem}\n__UPTIME__\n{uptime}\n__DF__\n{df}\n__NET__\n{net}\n__OS__\n{os}"
        );
        let mut stats = state.ingest(&output, std::time::Instant::now());
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

/// 单次采样与信号操作共用的超时：远端 `ps` 卡死不能拖住后续轮询。
const PROCESS_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(super) async fn run_process_sample(target: ProcessTarget) {
    let result = match &target.remote {
        Some(remote) => {
            match tokio::time::timeout(
                PROCESS_COMMAND_TIMEOUT,
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
                match tokio::time::timeout(
                    PROCESS_COMMAND_TIMEOUT,
                    remote.exec_command_capture(&command),
                )
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
        format!(
            "{action}超时，已等待 {} 秒",
            PROCESS_COMMAND_TIMEOUT.as_secs()
        ),
    )
}

async fn local_command_output(command: &str) -> Result<String, AppError> {
    let mut child = Command::new("sh");
    child.arg("-c").arg(command).kill_on_drop(true);
    match tokio::time::timeout(PROCESS_COMMAND_TIMEOUT, child.output()).await {
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
    let status = tokio::time::timeout(PROCESS_COMMAND_TIMEOUT, child.status())
        .await
        .map_err(|_| "本地 kill 命令超时".to_string())?
        .map_err(|error| format!("执行本地 kill 失败: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("本地 kill 返回失败状态: {status}"))
    }
}
