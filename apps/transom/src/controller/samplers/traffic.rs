//! 连接级网络流量采样器。
//!
//! 本地终端的系统级流量由 `local_host` 读取平台累计计数器后统一广播；
//! 本模块只负责 SSH/Telnet 等连接自身的传输层计数。

use super::common::send_overwritable;
use std::sync::Arc;
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
