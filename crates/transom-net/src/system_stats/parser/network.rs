//! 网卡累计字节与速率换算。

use std::time::Duration;
use transom_core::InterfaceRate;

/// 解析 `/proc/net/dev` 的 `__NET__` 段，返回非回环网卡的累计字节。
pub fn parse_net_dev_interfaces(content: &str) -> Vec<(String, u64, u64)> {
    let text = if let Some((_, rest)) = content.split_once("__NET__") {
        rest
    } else {
        content
    };
    let mut interfaces = Vec::with_capacity(4);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("__") || trimmed.starts_with("Inter-") {
            if trimmed.starts_with("__") && !interfaces.is_empty() {
                break;
            }
            continue;
        }
        let Some((name, values)) = trimmed.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name == "lo" {
            continue;
        }
        let mut fields = values.split_whitespace();
        let Some(receive) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let Some(transmit) = fields.nth(7).and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        interfaces.push((name.to_string(), receive, transmit));
    }
    interfaces
}

/// 按两次累计快照计算每块网卡的速率。
///
/// 新出现的网卡没有前值，本轮速率记 0；消失的网卡不输出。
pub fn calculate_interface_rates(
    previous: &[(String, u64, u64)],
    current: &[(String, u64, u64)],
    elapsed: Duration,
) -> Vec<InterfaceRate> {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return Vec::new();
    }
    let mut rates = Vec::with_capacity(current.len());
    for (name, rx, tx) in current {
        let previous_entry = previous.iter().find(|(previous, _, _)| previous == name);
        let (rx_before, tx_before) = previous_entry.map_or((*rx, *tx), |(_, rx, tx)| (*rx, *tx));
        let rate = |delta: u64| (delta as f64 / seconds).round() as u64;
        rates.push(InterfaceRate {
            name: name.clone(),
            rx_bytes_per_sec: rate(rx.saturating_sub(rx_before)),
            tx_bytes_per_sec: rate(tx.saturating_sub(tx_before)),
        });
    }
    rates
}
