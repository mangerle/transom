use crate::error::AppError;
use crate::ids::ForwardId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAddress {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SocksOptions {
    #[serde(default)]
    pub auth: SocksAuth,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone)]
pub struct ForwardStatus {
    pub rule_id: ForwardId,
    pub state: ForwardState,
    pub error: Option<AppError>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
