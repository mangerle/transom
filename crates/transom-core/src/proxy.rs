//! 出站代理配置：直连、SOCKS5 或 HTTP CONNECT。
//!
//! 设计原理（Why）：
//! - 受限内网与需要代理出站的办公网络里，SSH 客户端必须在建立物理 TCP
//!   连接之前先完成代理握手；把代理策略定义成带数据的枚举，让"是否带鉴权、
//!   走哪种协议"由类型表达，避免一堆互斥布尔标志；
//! - 会话级可覆盖全局：`Session::proxy == None` 表示跟随全局设置，用户既能
//!   设置公司统一出口，也能给单条会话指定专用跳板代理。
//! - 代价与局限：代理口令随配置持久化（与主机地址同处会话文件），界面不会
//!   回显明文；如需更高等级的凭据保护，应改用系统钥匙串（后续可平移）。

use serde::{Deserialize, Serialize};

/// 代理认证信息（RFC 1929 用户名/密码 或 HTTP Basic）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProxyAuth {
    pub username: String,
    #[serde(default)]
    pub password: String,
}

/// 出站代理策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutboundProxy {
    /// 直连目标，不经过任何代理。
    #[default]
    Direct,
    /// SOCKS5（RFC 1928），可选用户名/密码认证（RFC 1929）。
    Socks5 {
        #[serde(default)]
        host: String,
        #[serde(default)]
        port: u16,
        #[serde(default)]
        auth: Option<ProxyAuth>,
    },
    /// HTTP CONNECT 隧道，可选 Basic 认证。
    Http {
        #[serde(default)]
        host: String,
        #[serde(default)]
        port: u16,
        #[serde(default)]
        auth: Option<ProxyAuth>,
    },
}

impl OutboundProxy {
    /// 是否直连（不经过代理）。
    pub fn is_direct(&self) -> bool {
        matches!(self, Self::Direct)
    }

    /// 代理服务器地址；直连返回 `None`。
    pub fn server(&self) -> Option<(&str, u16)> {
        match self {
            Self::Direct => None,
            Self::Socks5 { host, port, .. } | Self::Http { host, port, .. } => {
                Some((host.as_str(), *port))
            }
        }
    }

    /// 连接前的配置校验。
    ///
    /// # Errors
    /// 代理主机为空、端口为 0 或用户名缺失时返回中文原因，供表单直接展示。
    pub fn validate(&self) -> Result<(), String> {
        let (host, port, auth) = match self {
            Self::Direct => return Ok(()),
            Self::Socks5 { host, port, auth } | Self::Http { host, port, auth } => {
                (host, *port, auth)
            }
        };
        if host.trim().is_empty() {
            return Err("代理服务器地址不能为空".to_string());
        }
        if port == 0 {
            return Err("代理服务器端口必须大于 0".to_string());
        }
        if let Some(auth) = auth {
            if auth.username.trim().is_empty() {
                return Err("代理用户名不能为空，或关闭代理认证".to_string());
            }
            if auth.password.is_empty() {
                return Err("代理密码不能为空，或关闭代理认证".to_string());
            }
        }
        Ok(())
    }

    /// 日志用描述：只含协议与地址，绝不含口令。
    pub fn describe(&self) -> String {
        match self {
            Self::Direct => "直连".to_string(),
            Self::Socks5 { host, port, .. } => format!("SOCKS5 {host}:{port}"),
            Self::Http { host, port, .. } => format!("HTTP {host}:{port}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_is_the_default_and_valid() {
        assert_eq!(OutboundProxy::default(), OutboundProxy::Direct);
        assert!(OutboundProxy::default().validate().is_ok());
        assert!(OutboundProxy::default().is_direct());
    }

    #[test]
    fn proxy_round_trips_through_json() {
        let proxy = OutboundProxy::Socks5 {
            host: "proxy.internal".into(),
            port: 1080,
            auth: Some(ProxyAuth {
                username: "ops".into(),
                password: "secret".into(),
            }),
        };
        let json = serde_json::to_string(&proxy).unwrap();
        assert_eq!(serde_json::from_str::<OutboundProxy>(&json).unwrap(), proxy);
    }

    /// 老配置文件没有代理字段时按直连降级，而不是解析失败。
    #[test]
    fn unknown_missing_fields_fall_back_to_defaults() {
        let proxy: OutboundProxy =
            serde_json::from_str(r#"{"kind":"socks5","host":"10.0.0.2"}"#).unwrap();
        assert_eq!(
            proxy,
            OutboundProxy::Socks5 {
                host: "10.0.0.2".into(),
                port: 0,
                auth: None,
            }
        );
        assert!(proxy.validate().is_err());
    }

    #[test]
    fn auth_requires_username_and_password() {
        let proxy = OutboundProxy::Http {
            host: "proxy".into(),
            port: 3128,
            auth: Some(ProxyAuth {
                username: String::new(),
                password: "x".into(),
            }),
        };
        assert!(proxy.validate().is_err());
        let proxy = OutboundProxy::Http {
            host: "proxy".into(),
            port: 3128,
            auth: Some(ProxyAuth {
                username: "u".into(),
                password: String::new(),
            }),
        };
        assert!(proxy.validate().is_err());
    }

    #[test]
    fn describe_never_leaks_credentials() {
        let proxy = OutboundProxy::Http {
            host: "proxy".into(),
            port: 3128,
            auth: Some(ProxyAuth {
                username: "user".into(),
                password: "top-secret".into(),
            }),
        };
        let description = proxy.describe();
        assert!(!description.contains("top-secret"));
        assert!(!description.contains("user"));
    }
}
