use crate::error::AppError;
use crate::ids::ForwardId;
use crate::model::session::Session;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForwardRule {
    pub id: ForwardId,
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub kind: ForwardKind,
}

impl ForwardRule {
    pub fn new(name: impl Into<String>, kind: ForwardKind) -> Self {
        Self {
            id: ForwardId::new(),
            name: name.into(),
            enabled: true,
            kind,
        }
    }

    /// 生成与本条规则等价的 `ssh -N` 命令行，供界面"复制 CLI"使用。
    ///
    /// 参数顺序按 `ssh(1)` 惯例：连接目标在前（`user@host -p port`），
    /// 转发参数紧随其后。端口为会话默认 22 时不写 `-p`，保持命令最短可用。
    pub fn ssh_command(&self, session: &Session) -> String {
        let (user_at_host, port_suffix) = match &session.connection {
            crate::Connection::Ssh(ssh) => {
                let target = format!("{}@{}", ssh.username, ssh.host);
                let port = if ssh.port == 22 {
                    String::new()
                } else {
                    format!(" -p {}", ssh.port)
                };
                (target, port)
            }
            _ => (String::new(), String::new()),
        };
        let forward = match &self.kind {
            ForwardKind::Local { bind, target } => {
                format!("-L {}:{}", bind_argument(bind), target_argument(target))
            }
            ForwardKind::Remote { bind, target } => {
                format!("-R {}:{}", bind_argument(bind), target_argument(target))
            }
            ForwardKind::Dynamic { bind, .. } => format!("-D {}", bind_argument(bind)),
        };
        let mut parts = vec![format!("ssh -N {forward}")];
        if !port_suffix.is_empty() {
            parts.push(port_suffix.trim_start().to_string());
        }
        if !user_at_host.is_empty() {
            parts.push(user_at_host);
        }
        parts.join(" ")
    }

    /// Local 转发在浏览器可直接打开的地址；`None` 表示不提供入口。
    ///
    /// 只服务 Local：Remote 的目标在本机侧但语义是"远端进来的访问"，
    /// Dynamic 的目标按连接决定，都没有稳定的浏览器入口。绑定地址为
    /// 通配（`0.0.0.0`/`::`/`*`/空）时映射回 `127.0.0.1`——浏览器连的
    /// 是本机回环，不是通配地址本身。协议猜测 `http` 而非 `https`：
    /// 猜错的代价是一张证书错误页，而要求用户手工补协议的代价更高。
    pub fn browser_url(&self) -> Option<String> {
        let ForwardKind::Local { bind, .. } = &self.kind else {
            return None;
        };
        let host = wildcard_bind_host(&bind.host);
        Some(format!("http://{}:{}", host, bind.port))
    }

    /// 是否与另一条规则争用同一个监听端口。
    ///
    /// 只有在本机监听的规则（Local/Dynamic）之间会冲突——Remote 的监听
    /// 发生在远端主机上，本机端口不复用；同一条规则自身不算冲突。
    pub fn conflicts_with(&self, other: &ForwardRule) -> bool {
        if self.id == other.id {
            return false;
        }
        match (self.listen_bind(), other.listen_bind()) {
            (Some(self_bind), Some(other_bind)) => {
                self_bind.host == other_bind.host && self_bind.port == other_bind.port
            }
            _ => false,
        }
    }

    /// 本机侧的监听端；Remote 的监听发生在远端，返回 `None`。
    pub fn listen_bind(&self) -> Option<&BindAddress> {
        match &self.kind {
            ForwardKind::Local { bind, .. } | ForwardKind::Dynamic { bind, .. } => Some(bind),
            ForwardKind::Remote { .. } => None,
        }
    }
}

/// 通配绑定地址的展示回环映射；普通地址原样返回。
fn wildcard_bind_host(host: &str) -> &str {
    match host.trim() {
        "" | "*" | "0.0.0.0" | "::" | "0:0:0:0:0:0:0:0" => "127.0.0.1",
        other => other,
    }
}

/// CLI 参数里的绑定端：地址为通配或空时写 `*:port`（ssh 的通配写法）。
fn bind_argument(bind: &BindAddress) -> String {
    let host = match bind.host.trim() {
        "" | "0.0.0.0" | "::" | "0:0:0:0:0:0:0:0" => "*".to_string(),
        other => other.to_string(),
    };
    format!("{host}:{}", bind.port)
}

/// CLI 参数里的目标端。
fn target_argument(target: &TargetAddress) -> String {
    format!("{}:{}", target.host, target.port)
}

/// 常用转发预设：一键填入新建表单，减少高频场景的手工输入。
///
/// 端口取各服务的默认监听端口；标签存 i18n 键而非文案（const 无法运行时
/// 查表），读取时经 `label_key()` 走 rust-i18n。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardPreset {
    pub label_key: &'static str,
    pub kind: ForwardPresetKind,
    pub listen_port: u16,
    pub target_port: u16,
}

/// 预设指向的转发类型；Dynamic 无目标端口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardPresetKind {
    Local,
    Dynamic,
}

impl ForwardPreset {
    /// 是否需要目标端字段。
    pub fn needs_target(self) -> bool {
        matches!(self.kind, ForwardPresetKind::Local)
    }
}

/// 四档内置预设。
pub const FORWARD_PRESETS: [ForwardPreset; 4] = [
    ForwardPreset {
        label_key: "forward.preset_mysql",
        kind: ForwardPresetKind::Local,
        listen_port: 3306,
        target_port: 3306,
    },
    ForwardPreset {
        label_key: "forward.preset_postgresql",
        kind: ForwardPresetKind::Local,
        listen_port: 5432,
        target_port: 5432,
    },
    ForwardPreset {
        label_key: "forward.preset_redis",
        kind: ForwardPresetKind::Local,
        listen_port: 6379,
        target_port: 6379,
    },
    ForwardPreset {
        label_key: "forward.preset_socks5",
        kind: ForwardPresetKind::Dynamic,
        listen_port: 1080,
        target_port: 1080,
    },
];

/// 单条转发隧道的实时计量：双向累计字节与累计连接数。
///
/// 设计原理（Why）：计数器是 `Arc<AtomicU64>` 对——`ForwardStatus` 事件把
/// 句柄随状态推给界面后，UI 每帧直接 `load` 原子值渲染，无需周期性事件流
/// 或轮询任务；`Relaxed` 序足够，计数只用于展示，不参与任何控制流判断。
/// 连接数按 winssh 语义取"累计连接数"（只加不减），活跃连接数没有低成本
/// 的精确口径，不值得为展示引入活跃集合。
#[derive(Clone, Default)]
pub struct ForwardMeter {
    bytes: Arc<AtomicU64>,
    connections: Arc<AtomicU64>,
}

impl ForwardMeter {
    /// 记录一次双向传输的字节增量。
    pub fn add_bytes(&self, value: u64) {
        self.bytes.fetch_add(value, Ordering::Relaxed);
    }

    /// 记录一条新建立的转发连接。
    pub fn add_connection(&self) {
        self.connections.fetch_add(1, Ordering::Relaxed);
    }

    /// 读取 `(累计字节, 累计连接数)` 快照。
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.bytes.load(Ordering::Relaxed),
            self.connections.load(Ordering::Relaxed),
        )
    }
}

impl PartialEq for ForwardMeter {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot() == other.snapshot()
    }
}

impl std::fmt::Debug for ForwardMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (bytes, connections) = self.snapshot();
        f.debug_struct("ForwardMeter")
            .field("bytes", &bytes)
            .field("connections", &connections)
            .finish()
    }
}

/// 把字节数格式化为人类可读流量（1024 进制）。
///
/// 阈值约定与 winssh 一致：小于 1 KiB 显示整数 B，逐级升单位；KB 一位小数、
/// MB 及以上两位小数——低位小数没有信息量，高位小数保留趋势可读性。
pub fn format_traffic(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value < KIB {
        format!("{bytes} B")
    } else if value < MIB {
        format!("{:.1} KB", value / KIB)
    } else if value < GIB {
        format!("{:.2} MB", value / MIB)
    } else {
        format!("{:.2} GB", value / GIB)
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ForwardKind {
    Local {
        bind: BindAddress,
        target: TargetAddress,
    },
    Remote {
        bind: BindAddress,
        target: TargetAddress,
    },
    Dynamic {
        bind: BindAddress,
        socks: SocksOptions,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindAddress {
    pub host: String,
    pub port: u16,
}

impl BindAddress {
    pub fn local(port: u16) -> Self {
        Self {
            host: "127.0.0.1".into(),
            port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetAddress {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SocksOptions {
    #[serde(default)]
    pub auth: SocksAuth,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SocksAuth {
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ForwardState {
    Stopped,
    Starting,
    Active {
        bound_port: Option<u16>,
    },
    Failed,
    /// 自动启动遇到公网监听地址，等待用户手动确认。
    NeedsConfirmation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForwardStatus {
    pub rule_id: ForwardId,
    pub state: ForwardState,
    pub error: Option<AppError>,
    /// 活跃隧道的实时计量句柄；UI 每帧读原子值渲染流量与连接数。
    /// 只有 `Active` 状态携带，停止/失败/待确认时为 `None`。
    pub meter: Option<ForwardMeter>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::session::SshConnection;

    fn local_rule(port: u16, target_port: u16) -> ForwardRule {
        ForwardRule::new(
            format!("local-{port}"),
            ForwardKind::Local {
                bind: BindAddress::local(port),
                target: TargetAddress {
                    host: "127.0.0.1".into(),
                    port: target_port,
                },
            },
        )
    }

    fn ssh_session() -> Session {
        Session::new(
            "prod",
            crate::Connection::Ssh(SshConnection {
                host: "prod.internal".into(),
                port: 22,
                username: "deploy".into(),
                auth: crate::AuthMethod::Agent,
                options: crate::SessionOptions::default(),
                proxy_jump: None,
                protection: crate::HostProtection::Normal,
            }),
        )
    }

    #[test]
    fn all_forward_kinds_round_trip() {
        let rules = vec![
            ForwardRule::new(
                "db",
                ForwardKind::Local {
                    bind: BindAddress::local(15432),
                    target: TargetAddress {
                        host: "db.internal".into(),
                        port: 5432,
                    },
                },
            ),
            ForwardRule::new(
                "web",
                ForwardKind::Remote {
                    bind: BindAddress {
                        host: "0.0.0.0".into(),
                        port: 8080,
                    },
                    target: TargetAddress {
                        host: "127.0.0.1".into(),
                        port: 3000,
                    },
                },
            ),
            ForwardRule::new(
                "proxy",
                ForwardKind::Dynamic {
                    bind: BindAddress::local(1080),
                    socks: SocksOptions::default(),
                },
            ),
        ];

        let json = serde_json::to_string(&rules).unwrap();
        let decoded: Vec<ForwardRule> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.len(), 3);
        assert!(decoded.iter().all(|rule| rule.enabled));
    }

    #[test]
    fn ssh_command_covers_all_kinds_and_custom_port() {
        let session = ssh_session();
        let local = local_rule(15432, 5432);
        assert_eq!(
            local.ssh_command(&session),
            "ssh -N -L 127.0.0.1:15432:127.0.0.1:5432 deploy@prod.internal"
        );

        let dynamic = ForwardRule::new(
            "proxy",
            ForwardKind::Dynamic {
                bind: BindAddress::local(1080),
                socks: SocksOptions::default(),
            },
        );
        assert_eq!(
            dynamic.ssh_command(&session),
            "ssh -N -D 127.0.0.1:1080 deploy@prod.internal"
        );

        let remote = ForwardRule::new(
            "web",
            ForwardKind::Remote {
                bind: BindAddress {
                    host: "0.0.0.0".into(),
                    port: 8080,
                },
                target: TargetAddress {
                    host: "127.0.0.1".into(),
                    port: 3000,
                },
            },
        );
        assert_eq!(
            remote.ssh_command(&session),
            "ssh -N -R *:8080:127.0.0.1:3000 deploy@prod.internal"
        );

        // 非 22 端口的会话补 -p 参数。
        let mut custom = ssh_session();
        if let crate::Connection::Ssh(ssh) = &mut custom.connection {
            ssh.port = 2222;
        }
        assert_eq!(
            local.ssh_command(&custom),
            "ssh -N -L 127.0.0.1:15432:127.0.0.1:5432 -p 2222 deploy@prod.internal"
        );
    }

    #[test]
    fn browser_url_maps_wildcard_binds_to_loopback() {
        let normal = local_rule(8080, 3000);
        assert_eq!(
            normal.browser_url().as_deref(),
            Some("http://127.0.0.1:8080")
        );

        let wildcard = ForwardRule::new(
            "wide",
            ForwardKind::Local {
                bind: BindAddress {
                    host: "0.0.0.0".into(),
                    port: 8080,
                },
                target: TargetAddress {
                    host: "127.0.0.1".into(),
                    port: 3000,
                },
            },
        );
        assert_eq!(
            wildcard.browser_url().as_deref(),
            Some("http://127.0.0.1:8080")
        );

        // Remote/Dynamic 没有稳定的浏览器入口。
        let remote = ForwardRule::new(
            "web",
            ForwardKind::Remote {
                bind: BindAddress::local(8080),
                target: TargetAddress {
                    host: "127.0.0.1".into(),
                    port: 3000,
                },
            },
        );
        assert_eq!(remote.browser_url(), None);
    }

    #[test]
    fn conflicts_only_between_local_listeners_on_same_bind() {
        let first = local_rule(8080, 3000);
        let second = local_rule(8080, 3306);
        assert!(first.conflicts_with(&second), "同端口 Local 规则冲突");

        let other_port = local_rule(8081, 3000);
        assert!(!first.conflicts_with(&other_port));

        // 自身不算冲突（就地编辑时排除自身用同一规则对象比较）。
        assert!(!first.conflicts_with(&first));

        // Remote 的监听在远端，与本机监听不争用。
        let remote = ForwardRule::new(
            "web",
            ForwardKind::Remote {
                bind: BindAddress::local(8080),
                target: TargetAddress {
                    host: "127.0.0.1".into(),
                    port: 3000,
                },
            },
        );
        assert!(!first.conflicts_with(&remote));

        let dynamic = ForwardRule::new(
            "proxy",
            ForwardKind::Dynamic {
                bind: BindAddress::local(8080),
                socks: SocksOptions::default(),
            },
        );
        assert!(
            first.conflicts_with(&dynamic),
            "Local 与 Dynamic 同端口冲突"
        );
    }

    #[test]
    fn meter_counts_bytes_and_connections() {
        let meter = ForwardMeter::default();
        assert_eq!(meter.snapshot(), (0, 0));
        meter.add_bytes(512);
        meter.add_bytes(512);
        meter.add_connection();
        meter.add_connection();
        assert_eq!(meter.snapshot(), (1024, 2));
    }

    #[test]
    fn format_traffic_matches_thresholds() {
        assert_eq!(format_traffic(0), "0 B");
        assert_eq!(format_traffic(1023), "1023 B");
        assert_eq!(format_traffic(1024), "1.0 KB");
        assert_eq!(format_traffic(1536), "1.5 KB");
        assert_eq!(format_traffic(1024 * 1024), "1.00 MB");
        assert_eq!(
            format_traffic(1024 * 1024 * 1024),
            "1.00 GB",
            "GB 以上保持两位小数"
        );
    }

    #[test]
    fn presets_cover_four_common_scenarios() {
        assert_eq!(FORWARD_PRESETS.len(), 4);
        assert!(FORWARD_PRESETS[0].needs_target(), "MySQL 预设是 Local 转发");
        assert_eq!(FORWARD_PRESETS[0].listen_port, 3306);
        assert!(!FORWARD_PRESETS[3].needs_target(), "SOCKS5 预设是动态转发");
        assert_eq!(FORWARD_PRESETS[3].listen_port, 1080);
    }
}
