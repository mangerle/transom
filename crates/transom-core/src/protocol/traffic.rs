//! 网络吞吐统计模型。
//!
//! SSH 会话的计数下沉到传输层（加密字节流）而不是终端通道：这样终端输入输出、
//! SFTP 传输、端口转发与心跳保活都会被统一计入，与用户对"当前连接网速"的
//! 直觉一致，也不会因为多开 SFTP 通道而漏算。
//!
//! 本地终端没有 SSH 传输层，改为读取 `/proc/net/dev` 汇总非回环网卡的累计
//! 收发字节，得到的是本机系统级网速；两者共用 [`NetworkTraffic`] 的差值换算。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 双向累计字节计数器。
///
/// 收发两个方向彼此独立，采样线程只做单调读取与差值计算，
/// 因此使用 `Relaxed` 内存序即可，无需引入互斥锁。
#[derive(Debug, Default)]
pub struct TrafficCounter {
    received_bytes: AtomicU64,
    sent_bytes: AtomicU64,
}

impl TrafficCounter {
    /// 记录从远端读取的字节数。
    pub fn add_received(&self, bytes: usize) {
        self.received_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 记录写入远端的字节数。
    pub fn add_sent(&self, bytes: usize) {
        self.sent_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 返回累计 `(接收字节, 发送字节)` 快照。
    pub fn totals(&self) -> (u64, u64) {
        (
            self.received_bytes.load(Ordering::Relaxed),
            self.sent_bytes.load(Ordering::Relaxed),
        )
    }
}

/// 汇总 `/proc/net/dev` 中所有非回环网卡的累计收发字节。
///
/// 返回顺序为 `(接收字节, 发送字节)`，与 [`TrafficCounter::totals`] 一致，供本地
/// 终端按差值换算系统级网速。只排除 `lo`：docker/veth/vpn 等虚拟接口是否重复
/// 计数取决于系统拓扑，这里不做过早猜测，保持与 `ifconfig` 总量口径一致。
///
/// 没有可解析的非回环接口时返回 `None`，调用方应保留上一次有效样本而不是
/// 发送零速率，避免网络采样偶发失败被误显示为"断网"。
pub fn parse_net_dev_totals(content: &str) -> Option<(u64, u64)> {
    let mut received = 0u64;
    let mut sent = 0u64;
    let mut found = false;
    for line in content.lines() {
        let Some((interface, values)) = line.split_once(':') else {
            continue;
        };
        if interface.trim().is_empty() || interface.trim() == "lo" {
            continue;
        }
        let mut fields = values.split_whitespace();
        let Some(receive_bytes) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        // 冒号后第 9 个字段是发送字节数；先消费接收字节，再跳过 7 个接收统计字段。
        let Some(transmit_bytes) = fields.nth(7).and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        received = received.saturating_add(receive_bytes);
        sent = sent.saturating_add(transmit_bytes);
        found = true;
    }
    found.then_some((received, sent))
}

/// 某一采样时刻的上下行速率快照，单位为字节/秒。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkTraffic {
    pub upload_bytes_per_sec: u64,
    pub download_bytes_per_sec: u64,
}

impl NetworkTraffic {
    /// 根据两次累计快照与采样间隔计算速率。
    ///
    /// 计数器回绕或连接重置时差值可能为负，统一用饱和减法归零；
    /// 间隔非法（为零）时返回全零，避免除零。
    pub fn from_delta(previous: (u64, u64), current: (u64, u64), elapsed: Duration) -> Self {
        let seconds = elapsed.as_secs_f64();
        if seconds <= 0.0 {
            return Self::default();
        }
        let per_second = |delta: u64| -> u64 {
            let rate = delta as f64 / seconds;
            if rate.is_finite() && rate > 0.0 {
                rate.round().min(u64::MAX as f64) as u64
            } else {
                0
            }
        };
        Self {
            download_bytes_per_sec: per_second(current.0.saturating_sub(previous.0)),
            upload_bytes_per_sec: per_second(current.1.saturating_sub(previous.1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_accumulates_both_directions() {
        let counter = TrafficCounter::default();
        assert_eq!(counter.totals(), (0, 0));
        counter.add_received(1024);
        counter.add_received(512);
        counter.add_sent(64);
        assert_eq!(counter.totals(), (1536, 64));
    }

    #[test]
    fn delta_converts_to_per_second_rates() {
        let traffic =
            NetworkTraffic::from_delta((1000, 2000), (3048, 4048), Duration::from_secs(2));
        assert_eq!(traffic.download_bytes_per_sec, 1024);
        assert_eq!(traffic.upload_bytes_per_sec, 1024);

        let one_second = NetworkTraffic::from_delta((0, 0), (1, 1), Duration::from_secs(1));
        assert_eq!(one_second.download_bytes_per_sec, 1);
        assert_eq!(one_second.upload_bytes_per_sec, 1);
    }

    #[test]
    fn delta_tolerates_reset_and_zero_elapsed() {
        let reset = NetworkTraffic::from_delta((5000, 5000), (10, 20), Duration::from_secs(1));
        assert_eq!(reset, NetworkTraffic::default());

        let zero = NetworkTraffic::from_delta((0, 0), (100, 100), Duration::ZERO);
        assert_eq!(zero, NetworkTraffic::default());
    }

    #[test]
    fn net_dev_totals_sum_non_loopback_interfaces() {
        let content = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 900 1 0 0 0 0 0 0 800 1 0 0 0 0 0 0
  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0
 wlan0: 300 3 0 0 0 0 0 0 400 4 0 0 0 0 0 0
";
        assert_eq!(parse_net_dev_totals(content), Some((1300, 2400)));
    }

    #[test]
    fn net_dev_totals_ignore_malformed_and_loopback_only() {
        let content = "\
  lo: 100 1 0 0 0 0 0 0 200 1 0 0 0 0 0 0
bad_line_without_colon
  eth0: not-a-number 1 0 0 0 0 0 0 2 0 0 0 0 0 0 0
  eth1: 1 0 0 0
";
        assert_eq!(parse_net_dev_totals(content), None);
    }
}
