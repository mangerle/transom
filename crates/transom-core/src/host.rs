//! 主机级网络动作契约：批量拨测与 Wake-on-LAN 的入参与结果类型。
//!
//! 设计原理（Why）：
//! - 实现初衷：这两件事在界面上是同一个入口（对会话列表里的若干主机做一次
//!   网络操作），在执行上是同一个并发管道（有界并发 + 逐条回报）。拆成两套
//!   命令/事件会让控制器多出一段几乎重复的 JoinSet 代码，因此收敛为一个
//!   `HostAction` 枚举；
//! - 核心优势：动作由界面侧从 `Session` 派生（`probe_for` / `wake_for`），
//!   控制器只认已经合法的 host/port/MAC，不在控制平面里重新解释会话结构；
//! - 代价与局限：只覆盖 TCP 握手探测与魔法包发送，不做 ICMP（需要 root 或
//!   非特权 ping 组，跨发行版行为不一致），也无法确认 WoL 是否真的唤醒了
//!   机器——`WakeSent` 的语义上限就是"包已发出"。

use crate::ids::SessionId;
use crate::session::{Connection, Session};

/// 一次主机级网络动作；`session_id` 用于把结果回填到界面表格的对应行。
#[derive(Debug, Clone)]
pub enum HostAction {
    /// TCP 连接探测：只做三次握手，成功后立即断开，不发送任何协议报文。
    Probe {
        session_id: SessionId,
        host: String,
        port: u16,
    },
    /// 发送 Wake-on-LAN 魔法包；`mac` 已在界面侧规范化为 `aa:bb:...`。
    Wake { session_id: SessionId, mac: String },
}

/// 动作结果。拨测与唤醒共用一套：界面按"会话 → 结果"渲染一行。
#[derive(Debug, Clone)]
pub enum HostOutcome {
    /// 端口可达，`latency_ms` 为 TCP 握手耗时（饱和到 u64 毫秒）。
    Reachable { latency_ms: u64 },
    /// 不可达或探测失败：`error` 携带可展示的原因（超时、拒绝、解析失败）。
    Unreachable { error: crate::AppError },
    /// 魔法包已发出；不代表网卡已唤醒目标机器。
    WakeSent,
}

impl HostAction {
    /// 从会话派生拨测动作；本地 Shell 与串口没有网络地址，返回 `None`。
    ///
    /// Telnet 与 SSH 一样按"主机 + 端口"探测：拨测回答的是"这台机器上的这个
    /// 端口通不通"，与上层协议无关。
    pub fn probe_for(session: &Session) -> Option<Self> {
        let (host, port) = match &session.connection {
            Connection::Ssh(ssh) => (ssh.host.clone(), ssh.port),
            Connection::Telnet(telnet) => (telnet.host.clone(), telnet.port),
            _ => return None,
        };
        if host.trim().is_empty() {
            return None;
        }
        Some(Self::Probe {
            session_id: session.id,
            host,
            port,
        })
    }

    /// 从会话派生唤醒动作；未配置 MAC 时返回 `None`，由界面给出明确提示。
    ///
    /// 只有 SSH 会话可唤醒：唤醒的是跑通用操作系统的服务器/工作站，Telnet
    /// 面向的是网络设备，两者很少需要 WoL，为此把 `wol_mac` 只放进 SSH 的
    /// `SessionOptions`，避免同义字段在两套 options 里各写一遍。
    pub fn wake_for(session: &Session) -> Option<Self> {
        let Connection::Ssh(ssh) = &session.connection else {
            return None;
        };
        let mac = ssh.options.wol_mac.as_deref().unwrap_or("").trim();
        if mac.is_empty() {
            return None;
        }
        Some(Self::Wake {
            session_id: session.id,
            mac: mac.to_string(),
        })
    }

    /// 动作归属的会话；结果事件按它回填界面行。
    pub fn session_id(&self) -> SessionId {
        match self {
            Self::Probe { session_id, .. } | Self::Wake { session_id, .. } => *session_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{LocalConnection, SshConnection, TelnetConnection};

    fn ssh_session(mac: Option<String>) -> Session {
        let mut ssh = SshConnection::new("10.0.0.8", "deploy");
        ssh.options.wol_mac = mac;
        Session::new("web", Connection::Ssh(ssh))
    }

    #[test]
    fn probe_derives_host_and_port_per_protocol() {
        let probe = HostAction::probe_for(&ssh_session(None)).unwrap();
        assert!(matches!(
            probe,
            HostAction::Probe { host, port: 22, .. } if host == "10.0.0.8"
        ));

        let telnet = Session::new(
            "bbs",
            Connection::Telnet(TelnetConnection::new("bbs.example")),
        );
        let probe = HostAction::probe_for(&telnet).unwrap();
        assert!(matches!(probe, HostAction::Probe { port: 23, .. }));

        // 本地 Shell 与串口没有可探测的网络地址。
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        assert!(HostAction::probe_for(&local).is_none());
    }

    #[test]
    fn probe_skips_sessions_without_host() {
        let mut session = ssh_session(None);
        if let Connection::Ssh(ssh) = &mut session.connection {
            ssh.host = "   ".to_string();
        }
        assert!(HostAction::probe_for(&session).is_none());
    }

    #[test]
    fn wake_requires_configured_mac() {
        assert!(HostAction::wake_for(&ssh_session(None)).is_none());
        assert!(HostAction::wake_for(&ssh_session(Some(String::new()))).is_none());
        let action = HostAction::wake_for(&ssh_session(Some("aa:bb:cc:dd:ee:ff".to_string())));
        assert!(matches!(
            action,
            Some(HostAction::Wake { mac, .. }) if mac == "aa:bb:cc:dd:ee:ff"
        ));
    }
}
