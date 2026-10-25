//! 主机级网络动作执行：TCP 握手探测与 Wake-on-LAN 魔法包。
//!
//! 设计原理（Why）：
//! - 拨测选 TCP 连接而不是 ICMP：ICMP 需要 root 或 `net.ipv4.ping_group_range`
//!   授权，跨发行版行为不一致且无法打包成用户态能力；而运维真正关心的问题是
//!   "这台机器的这个端口能不能连上"，TCP 握手直接回答它，顺带把 DNS 解析耗时
//!   也算进延迟里，与真实连接体验同口径；
//! - 唤醒同时向 7 与 9 两个端口发包：网卡固件对端口的实现不统一（PC 多为 9，
//!   部分服务器 BMC 监听 7），多发一次的代价只是一个 102 字节的 UDP 数据报；
//! - 代价与局限：魔法包发进本机链路后由交换机泛洪，跨网段唤醒需要路由器开启
//!   定向广播或用子网掩码广播地址，本模块按全局广播 `255.255.255.255` 发送，
//!   不覆盖跨 VLAN 场景；也无法确认网卡是否真的把机器叫醒，`WakeSent` 的语义
//!   上限就是"包已发出"。

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::net::{TcpStream, UdpSocket};
use transom_core::MacAddress;

use crate::error::NetError;

/// Wake-on-LAN 约定端口：echo(7) 与 discard(9)。
const WOL_PORTS: [u16; 2] = [7, 9];

/// 魔法包同步前导：6 个 0xFF。
const WOL_SYNC: [u8; 6] = [0xff; 6];

/// 目标网卡地址在同步流之后重复的次数（规范值 16）。
const WOL_REPEAT: usize = 16;

/// 探测单个 TCP 端口，返回握手耗时（含 DNS 解析）。
///
/// 只建连不通信：握手成功即刻断开，不发送任何协议报文，因此对被测主机无副作用。
///
/// # Errors
///
/// - 超时：[`NetError::ProbeTimeout`]
/// - DNS 解析失败、连接被拒等：[`NetError::Connect`]
pub async fn probe_tcp(host: &str, port: u16, timeout: Duration) -> Result<Duration, NetError> {
    let started = Instant::now();
    let attempt = TcpStream::connect((host, port));
    match tokio::time::timeout(timeout, attempt).await {
        Ok(Ok(_stream)) => Ok(started.elapsed()),
        Ok(Err(source)) => Err(NetError::Connect {
            host: host.to_string(),
            port,
            source,
        }),
        Err(_elapsed) => Err(NetError::ProbeTimeout {
            host: host.to_string(),
            port,
            timeout_ms: timeout.as_millis() as u64,
        }),
    }
}

/// 向目标网卡的 MAC 发送 Wake-on-LAN 魔法包。
///
/// # Errors
///
/// MAC 无法解析、无可用广播路由或套接字创建失败时返回 [`NetError::Wake`]；
/// 两个约定端口只要有一次发送成功即视为已唤醒投递。
pub async fn send_wake_on_lan(mac: &str) -> Result<(), NetError> {
    let address = MacAddress::from_str(mac).map_err(|error| NetError::Wake {
        mac: mac.to_string(),
        reason: error.to_string(),
    })?;
    let packet = magic_packet(&address.octets());
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|source| wake_io_error(mac, source))?;
    socket
        .set_broadcast(true)
        .map_err(|source| wake_io_error(mac, source))?;

    let mut failures: Vec<String> = Vec::with_capacity(WOL_PORTS.len());
    for port in WOL_PORTS {
        let target = SocketAddr::from((Ipv4Addr::BROADCAST, port));
        if let Err(source) = socket.send_to(&packet, target).await {
            failures.push(format!("{port}: {source}"));
        }
    }
    if failures.len() == WOL_PORTS.len() {
        return Err(NetError::Wake {
            mac: mac.to_string(),
            reason: failures.join("；"),
        });
    }
    Ok(())
}

/// 组装 102 字节魔法包：6 字节全 1 同步流 + 目标 MAC 重复 16 次。
fn magic_packet(octets: &[u8; 6]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(WOL_SYNC.len() + octets.len() * WOL_REPEAT);
    packet.extend_from_slice(&WOL_SYNC);
    for _ in 0..WOL_REPEAT {
        packet.extend_from_slice(octets);
    }
    packet
}

fn wake_io_error(mac: &str, source: std::io::Error) -> NetError {
    NetError::Wake {
        mac: mac.to_string(),
        reason: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次性回环监听：返回端口与 listener 本体，`listener` Drop 即等价于端口关闭。
    ///
    /// 不用"listen 但不 accept"来模拟超时——Linux 内核会替未完成三次握手的
    /// SYN 回 SYN-ACK，连接照样成功；超时分枝改由错误映射测试覆盖（见
    /// `error::tests`），避免把网络环境写进断言。
    async fn listen_on_loopback() -> (u16, tokio::net::TcpListener) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        (listener.local_addr().unwrap().port(), listener)
    }

    /// 魔法包总长 102 字节：6 字节同步流 + 96 字节目标地址重复体。
    #[test]
    fn magic_packet_layout_matches_spec() {
        let octets = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let packet = magic_packet(&octets);
        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[..6], &WOL_SYNC);
        let body = &packet[6..];
        assert_eq!(body.len(), octets.len() * WOL_REPEAT);
        for (index, chunk) in body.chunks(octets.len()).enumerate() {
            assert_eq!(chunk, octets, "第 {index} 份重复体应为目标 MAC");
        }
    }

    #[tokio::test]
    async fn probe_reports_latency_for_open_port() {
        let (port, _listener) = listen_on_loopback().await;
        let latency = probe_tcp("127.0.0.1", port, Duration::from_secs(2))
            .await
            .expect("回环开放端口应判定可达");
        // 回环握手是亚毫秒级，上限只需挡住单位错误（秒当成毫秒之类）。
        assert!(latency.as_millis() < 1000, "耗时异常: {latency:?}");
    }

    #[tokio::test]
    async fn probe_reports_failure_for_closed_port() {
        let (port, listener) = listen_on_loopback().await;
        drop(listener);
        let error = probe_tcp("127.0.0.1", port, Duration::from_secs(2))
            .await
            .expect_err("已关闭的端口不应判定可达");
        assert!(matches!(error, NetError::Connect { .. }));
        let app_error = transom_core::AppError::from(error);
        assert_eq!(app_error.code, transom_core::ErrorCode::ConnectionFailed);
    }

    /// 非法 MAC 在发包前就被拒绝：不能把无法解析的字符串编成包扔进网络。
    #[tokio::test]
    async fn wake_rejects_invalid_mac() {
        let error = send_wake_on_lan("not-a-mac")
            .await
            .expect_err("非法 MAC 应报错");
        assert!(matches!(error, NetError::Wake { .. }));
        assert!(error.to_string().contains("not-a-mac"));
    }

    /// 广播路径不可在回环上验证收包，这里确认发送不失败即可：
    /// 套接字创建、`set_broadcast` 与两个端口的 `send_to` 全链路走通。
    #[tokio::test]
    async fn wake_sends_broadcast_without_error() {
        send_wake_on_lan("aa:bb:cc:dd:ee:ff")
            .await
            .expect("用户态广播发送不应失败");
    }
}
