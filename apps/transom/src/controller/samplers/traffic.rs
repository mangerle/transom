//! 网络流量采样器（远程连接与本地单机共享采样）。

use super::common::send_overwritable;
use std::sync::Arc;
use transom_core::protocol::traffic::parse_net_dev_totals;
use transom_core::{AppEvent, ConnectionId, HostAppEvent, NetworkTraffic, TabId, TrafficCounter};

pub(crate) fn spawn_traffic_sampler(
    traffic: Arc<TrafficCounter>,
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // 先交一个零样本，让网络徽标在连接建立后立刻出现；首个真实速率
        // 仍从完整的一秒窗口开始计算，避免瞬时大数字。
        // 零样本同样按可覆盖事件投递：丢它只影响徽标出现时机。
        if !send_overwritable(
            &event_tx,
            HostAppEvent::NetworkTrafficUpdated {
                tab_id,
                connection_id,
                traffic: NetworkTraffic::default(),
            }
            .into(),
            "网络流量",
        ) {
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
            if !send_overwritable(
                &event_tx,
                HostAppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: sample,
                }
                .into(),
                "网络流量",
            ) {
                break;
            }
        }
    })
}

/// 本地终端的网络采样订阅表：与共享系统统计同一模式，按机器计一份。
pub(crate) type LocalTrafficTabs = Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>;

/// 本地终端的网络采样（机器级共享）：读取本机 `/proc/net/dev` 的非回环
/// 网卡累计字节，广播给全部已登记标签。
///
/// 设计原理（Why）：本地网络流量是整台机器的属性（与 SSH 传输层计数不同），
/// 多个本地标签各自读取同一份 /proc/net/dev 只会得到 N 份相同数据；共享
/// 采样按机器计一次读取，结果逐标签广播。
/// 界面上按会话类型区分提示文案由渲染层负责。读取失败时本轮跳过并保留
/// 上一次有效样本，避免把一次文件读取抖动显示成断网。
pub(crate) async fn run_local_traffic_sampler(
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
        // 采样节拍：每次迭代等待 1 秒。置于循环头部可统一步调，
        // 确保正常广播与所有 continue 分支均按完整采样窗口等待，彻底杜绝无间隔热循环。
        interval.tick().await;
        // 持锁只做纯内存克隆，不跨 await；订阅清空即退出，
        // 下一个本地连接会重新拉起本采样器。
        let subscribers = match tabs.lock() {
            Ok(tabs) if tabs.is_empty() => break,
            Ok(tabs) => tabs.clone(),
            Err(_) => break,
        };
        let Some(current) = read_totals().await else {
            // 读取失败本轮跳过并保留上一次有效样本，由循环头部统一等待下一个采样窗口。
            continue;
        };
        let now = std::time::Instant::now();
        let elapsed = now.saturating_duration_since(previous_at);
        previous_at = now;
        let Some(previous_totals) = previous.replace(current) else {
            // 首轮没有可比较的基准样本：同样由循环头部统一等待下一个采样窗口。
            continue;
        };
        let sample = NetworkTraffic::from_delta(previous_totals, current, elapsed);
        let mut channel_closed = false;
        for (tab_id, connection_id) in subscribers {
            if !send_overwritable(
                &event_tx,
                HostAppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: sample,
                }
                .into(),
                "网络流量",
            ) {
                channel_closed = true;
                break;
            }
        }
        if channel_closed {
            break;
        }
    }
}
