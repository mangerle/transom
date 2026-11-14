//! 主机级网络动作契约：批量拨测的入参、派生与结果类型。
//!
//! 设计原理（Why）：
//! - 动作由界面侧从 `Session` 派生（`probe_for`），控制器只认已经合法的
//!   host/port，不在控制平面里重新解释会话结构，也就不需要为"哪些会话可拨测"
//!   在两侧各写一套判断；
//! - 代价与局限：只做 TCP 握手探测，不做 ICMP（需要 root 或非特权 ping 组，
//!   跨发行版行为不一致）。
//!
//! Wake-on-LAN 的界面入口已按产品决策移除（会话列表只保留拨测），本模块因此不再
//! 携带唤醒动作；网络层 `transom_net::send_wake_on_lan` 与 `SessionOptions::wol_mac`
//! 仍然保留，前者是已单测的公开能力，后者是既有会话文件里的用户数据。

use crate::ids::SessionId;
use crate::model::session::{Connection, Session};

/// 拨测协议口径：区分只需 TCP 握手的通用端口与需校验协议标识的 SSH 端口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// SSH 会话：TCP 建连后校验服务端主动下发的 `SSH-` 版本标识，
    /// 避免 TUN 虚拟网卡或透明代理在本机代答 SYN-ACK 造成假可达。
    Ssh,
    /// 纯 TCP 端口探测（如 Telnet）：只做三次握手，不等待服务端首包。
    Tcp,
}

/// 一次主机级网络动作；`session_id` 用于把结果回填到会话列表的对应行。
#[derive(Debug, Clone)]
pub enum HostAction {
    /// 主机连通性探测：按 `kind` 执行纯 TCP 握手或 SSH 协议标识校验。
    Probe {
        session_id: SessionId,
        host: String,
        port: u16,
        kind: ProbeKind,
    },
}

/// 动作结果；界面按"会话 → 结果"渲染一个状态点。
#[derive(Debug, Clone)]
pub enum HostOutcome {
    /// 端口可达，`latency_ms` 为 TCP 握手耗时（饱和到 u64 毫秒）。
    Reachable { latency_ms: u64 },
    /// 不可达或探测失败：`error` 携带可展示的原因（超时、拒绝、解析失败）。
    Unreachable { error: crate::AppError },
}

impl HostAction {
    /// 从会话派生拨测动作；本地 Shell 与串口没有网络地址，返回 `None`。
    ///
    /// SSH 派生为 [`ProbeKind::Ssh`]（校验服务端 Banner），Telnet 派生为
    /// [`ProbeKind::Tcp`]（仅做三次握手）。
    pub fn probe_for(session: &Session) -> Option<Self> {
        let (host, port, kind) = match &session.connection {
            Connection::Ssh(ssh) => (ssh.host.clone(), ssh.port, ProbeKind::Ssh),
            Connection::Telnet(telnet) => (telnet.host.clone(), telnet.port, ProbeKind::Tcp),
            _ => return None,
        };
        if host.trim().is_empty() {
            return None;
        }
        Some(Self::Probe {
            session_id: session.id,
            host,
            port,
            kind,
        })
    }

    /// 动作归属的会话；结果事件按它回填界面行。
    pub fn session_id(&self) -> SessionId {
        match self {
            Self::Probe { session_id, .. } => *session_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::session::{LocalConnection, SshConnection, TelnetConnection};

    fn ssh_session() -> Session {
        Session::new(
            "web",
            Connection::Ssh(SshConnection::new("10.0.0.8", "deploy")),
        )
    }

    #[test]
    fn probe_derives_host_and_port_per_protocol() {
        let probe = HostAction::probe_for(&ssh_session()).unwrap();
        assert!(matches!(
            probe,
            HostAction::Probe {
                host,
                port: 22,
                kind: ProbeKind::Ssh,
                ..
            } if host == "10.0.0.8"
        ));

        let telnet = Session::new(
            "bbs",
            Connection::Telnet(TelnetConnection::new("bbs.example")),
        );
        let probe = HostAction::probe_for(&telnet).unwrap();
        assert!(matches!(
            probe,
            HostAction::Probe {
                port: 23,
                kind: ProbeKind::Tcp,
                ..
            }
        ));

        // 本地 Shell 与串口没有可探测的网络地址。
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        assert!(HostAction::probe_for(&local).is_none());
    }

    #[test]
    fn probe_skips_sessions_without_host() {
        let mut session = ssh_session();
        if let Connection::Ssh(ssh) = &mut session.connection {
            ssh.host = "   ".to_string();
        }
        assert!(HostAction::probe_for(&session).is_none());
    }
}
