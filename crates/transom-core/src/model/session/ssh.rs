//! SSH 连接、认证方式、会话选项与跳板机模型。

use crate::secrets::ConnectionSecrets;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// ProxyJump 跳板机引用：目标会话经由哪一条已有 SSH 会话转发。
///
/// 只保存跳板会话的 ID 而不复制其连接参数：跳板机的地址、账号与凭据始终
/// 以目录里的那条会话为准，改一处即对全部依赖它的目标生效。
/// 老会话文件没有该字段，按直连降级。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyJumpConfig {
    pub jump_session_id: crate::ids::SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConnection {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
    #[serde(default)]
    pub options: SessionOptions,
    /// 跳板机引用；`None` 表示直连。链路可多层嵌套，环检测在连接前完成。
    #[serde(default)]
    pub proxy_jump: Option<ProxyJumpConfig>,
}

impl SshConnection {
    pub fn new(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 22,
            username: username.into(),
            auth: AuthMethod::Agent,
            options: SessionOptions::default(),
            proxy_jump: None,
        }
    }
}

/// 跳板链中的一个已解析节点：连接参数与本次连接可用的凭据。
///
/// 凭据随节点一起传递而不在 net 层回查钥匙串：net 是纯网络层，不应理解
/// 会话目录与钥匙串归属；界面侧在派发连接前已经按会话（或凭据模板）取好。
pub struct SshHop {
    pub connection: SshConnection,
    pub secrets: ConnectionSecrets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthMethod {
    Password {
        #[serde(default)]
        save: bool,
    },
    PrivateKey {
        path: PathBuf,
        #[serde(default)]
        save_passphrase: bool,
        /// 企业 CA 签发的 OpenSSH 用户证书（`*-cert.pub`）。
        ///
        /// 放在 `PrivateKey` 变体而不是连接选项上：证书只在"用这把私钥认证"时才有意义，
        /// 让类型替我们排除"密码认证却挂着证书"这种非法组合。
        /// 认证时优先走证书，被服务器拒绝再退回裸公钥认证（老服务器没配 CA 时不至于连不上）。
        #[serde(default)]
        cert_path: Option<PathBuf>,
    },
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOptions {
    pub connect_timeout_secs: u64,
    /// `0` disables keepalive messages.
    pub keepalive_secs: u64,
    pub allow_keyboard_interactive: bool,
    /// 意外断线后自动重连（退避 2s/5s/10s，上限 3 次）。
    /// 老会话文件没有该字段，按关闭降级。
    #[serde(default)]
    pub auto_reconnect: bool,
    /// 是否启用 SSH Agent 代理转发（对应 ssh -A）。
    /// 在多跳堡垒机场景下将本地 SSH Agent 安全转发至下游远端主机。
    #[serde(default)]
    pub agent_forward: bool,
    /// 关机主机的网卡 MAC（规范化为 `aa:bb:cc:dd:ee:ff`），供 Wake-on-LAN 使用。
    ///
    /// 存文本而非新类型：单个手写坏值只会让这一条会话的唤醒失败并给出明确
    /// 错误，而不是把整个会话库的反序列化打挂。老会话文件没有该字段，按未配置降级。
    #[serde(default)]
    pub wol_mac: Option<String>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            keepalive_secs: 30,
            allow_keyboard_interactive: true,
            auto_reconnect: false,
            agent_forward: false,
            wol_mac: None,
        }
    }
}
