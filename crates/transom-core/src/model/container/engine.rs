//! 引擎引用与连接参数：种类、端点、配置档案与会话连接形态。

use crate::ids::EngineProfileId;
use crate::model::session_appearance::{SessionColor, SessionIcon};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// 容器引擎种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerEngineKind {
    /// Docker Engine 或与 Docker 兼容的发行版。
    #[default]
    Docker,
    /// Podman（走 Docker 兼容 REST 路径）。
    Podman,
}

impl ContainerEngineKind {
    /// 界面与日志使用的稳定名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
        }
    }

    /// 本机 CLI 兜底时优先尝试的可执行文件名。
    pub fn cli_program(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }

    /// 端点语法前缀（`unix://`、`npipe://`、`ssh://` 等）。
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

impl fmt::Display for ContainerEngineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// 可保存的引擎连接配置（容器表单的产物）。
///
/// 设计原理（Why）：连接配置与终端会话分开建模——一个引擎连接打开的是
/// 管理工作区，而不是一条终端会话；`updated_at` 让备份/云同步沿用会话的
/// 时间戳裁决规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerEngineProfile {
    pub id: EngineProfileId,
    pub name: String,
    #[serde(default)]
    pub engine: ContainerEngineRef,
    /// 所属引擎分组；`None` 表示根层级。
    #[serde(default)]
    pub folder_id: Option<crate::ids::FolderId>,
    /// 用户选择的展示图标；`Auto` 跟随引擎类型。
    #[serde(default)]
    pub icon: SessionIcon,
    /// 环境色；`None` 表示使用主题色。
    #[serde(default)]
    pub color: SessionColor,
    /// 最后修改时间（Unix 毫秒），供多端合并判定谁更新。
    #[serde(default)]
    pub updated_at: u64,
}

impl ContainerEngineProfile {
    pub fn new(name: impl Into<String>, engine: ContainerEngineRef) -> Self {
        Self {
            id: EngineProfileId::new(),
            name: name.into(),
            engine,
            folder_id: None,
            icon: SessionIcon::Auto,
            color: SessionColor::None,
            updated_at: now_millis(),
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now_millis();
    }

    /// 界面标题：优先用户命名，空名回落到引擎摘要。
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            self.engine.describe()
        } else {
            name.to_string()
        }
    }
}

/// 当前 Unix 毫秒时间戳；时钟异常时按 0 处理。
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

/// TCP TLS 配置：用于 Docker/Podman 守护进程开启 TLS 监听的远端连接。
///
/// 三个文件路径均为宿主机本地绝对路径；留空表示不加载对应证书，
/// `ca_cert` 留空等价于不验证服务端身份（仅开发/测试场景）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    /// CA 证书文件路径，用于验证服务端证书链。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ca_cert: String,
    /// 客户端证书文件路径，用于双向 TLS 认证。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_cert: String,
    /// 客户端私钥文件路径，与 `client_cert` 配对使用。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_key: String,
}

impl TlsConfig {
    /// 是否为空配置（三个路径均未填写）。
    pub fn is_empty(&self) -> bool {
        self.ca_cert.is_empty() && self.client_cert.is_empty() && self.client_key.is_empty()
    }
}

/// 引擎接入端点。
///
/// 序列化后随会话落盘，因此字段必须保持向后兼容：新增变体时老版本会因
/// 未知 `kind` 拒绝整份目录，升级前需要评估迁移策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContainerEndpoint {
    /// 由本机按平台与引擎种类自动发现。
    #[default]
    Auto,
    /// Unix 域套接字（Linux/macOS）。
    Unix { path: PathBuf },
    /// Windows 命名管道，如 `//./pipe/docker_engine`。
    NamedPipe { path: String },
    /// TCP 端点（如 `tcp://127.0.0.1:2375`）；支持可选 TLS。
    Tcp {
        url: String,
        /// TLS 配置；`None` 表示明文连接。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tls: Option<TlsConfig>,
    },
    /// 远端守护进程，通过 SSH 隧道建立连接（`docker system dial-stdio`）。
    Ssh {
        /// 远端主机地址（IP 或域名），不含端口。
        host: String,
        /// SSH 端口；`None` 时使用默认的 22。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// 登录用户名；`None` 时使用当前系统用户名。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        /// 私钥文件路径；`None` 时按 SSH 默认规则（`~/.ssh/id_*`）查找。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity_file: Option<String>,
        /// 登录密码；若配置则优先使用密码认证。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        /// 是否使用 sudo 提权执行容器命令。
        #[serde(default)]
        use_sudo: bool,
        /// sudo 密码；若启用 sudo 且未提供此字段，则默认复用登录密码。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sudo_password: Option<String>,
    },
    /// WSL 发行版内的 CLI 兜底。
    Wsl { distro: String },
    /// WSLC（WSL containers）CLI 兜底。
    Wslc,
    /// 任意 CLI 兜底：`program` 加固定前缀参数。
    Cli { program: String, args: Vec<String> },
}

impl ContainerEndpoint {
    /// 日志与界面摘要；不含任何凭证信息。
    pub fn describe(&self) -> String {
        match self {
            Self::Auto => "自动发现".to_string(),
            Self::Unix { path } => format!("unix://{}", path.display()),
            Self::NamedPipe { path } => format!("npipe:{}", display_pipe_path(path)),
            Self::Tcp { url, tls } => {
                if tls.is_some() {
                    format!("{url} (TLS)")
                } else {
                    url.clone()
                }
            }
            Self::Ssh {
                host, port, user, ..
            } => {
                let user_prefix = user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
                let port_suffix = port.map(|p| format!(":{p}")).unwrap_or_default();
                format!("ssh://{user_prefix}{host}{port_suffix}")
            }
            Self::Wsl { distro } => format!("wsl://{distro}"),
            Self::Wslc => "wslc://default".to_string(),
            Self::Cli { program, .. } => format!("cli://{program}"),
        }
    }

    /// 是否为 CLI 后端：能力与 resize 支持都和 HTTP 后端不同。
    pub fn is_cli(&self) -> bool {
        matches!(self, Self::Wsl { .. } | Self::Wslc | Self::Cli { .. })
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// 当前平台是否支持该端点；用于表单在保存前给出明确提示。
    ///
    /// # Errors
    /// 端点与宿主平台不匹配（如在 Linux 上选择命名管道）时返回中文原因。
    pub fn ensure_supported(&self) -> Result<(), String> {
        match self {
            Self::NamedPipe { .. } if !cfg!(windows) => Err("命名管道仅支持 Windows".to_string()),
            Self::Wsl { .. } | Self::Wslc if !cfg!(windows) => {
                Err("WSL/WSLC 端点仅支持 Windows".to_string())
            }
            _ => Ok(()),
        }
    }

    /// 获取唤起系统浏览器访问端口时的主机地址（Host）。
    ///
    /// 本地 Unix Socket、命名管道或 Auto 默认返回 "localhost"；
    /// 远端 TCP 端点提取其主机名或 IP；
    /// 远端 SSH 端点提取其 host；
    /// WSL 默认返回 "localhost"。
    pub fn browser_host(&self) -> String {
        match self {
            Self::Tcp { url, .. } => {
                let trimmed = url.trim();
                let without_scheme = if let Some(rest) = trimmed.strip_prefix("tcp://") {
                    rest
                } else if let Some(rest) = trimmed.strip_prefix("http://") {
                    rest
                } else if let Some(rest) = trimmed.strip_prefix("https://") {
                    rest
                } else {
                    trimmed
                };
                let host = without_scheme.split([':', '/']).next().unwrap_or("").trim();
                if host != "0.0.0.0" && !host.is_empty() {
                    host.to_string()
                } else {
                    "localhost".to_string()
                }
            }
            Self::Ssh { host, .. } => {
                let h = host.trim();
                if h != "0.0.0.0" && !h.is_empty() {
                    h.to_string()
                } else {
                    "localhost".to_string()
                }
            }
            _ => "localhost".to_string(),
        }
    }
}

/// 去掉 Windows 命名管道前缀里的反斜杠转义，只用于展示。
fn display_pipe_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// 会话与命令携带的引擎引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerEngineRef {
    pub kind: ContainerEngineKind,
    #[serde(default)]
    pub endpoint: ContainerEndpoint,
}

impl ContainerEngineRef {
    pub fn auto(kind: ContainerEngineKind) -> Self {
        Self {
            kind,
            endpoint: ContainerEndpoint::Auto,
        }
    }

    pub fn explicit(kind: ContainerEngineKind, endpoint: ContainerEndpoint) -> Self {
        Self { kind, endpoint }
    }

    /// 获取宿主浏览器访问端口时的目标主机地址。
    pub fn browser_host(&self) -> String {
        self.endpoint.browser_host()
    }

    /// 界面摘要：`Docker（自动发现）`、`Podman（unix:///run/...）`。
    pub fn describe(&self) -> String {
        format!("{}（{}）", self.kind.label(), self.endpoint.describe())
    }
}

/// 容器 exec 参数：在容器内打开交互式终端。
///
/// 工作区已经持有引擎会话，因此这里不再重复携带引擎端点；它只是一份
/// “在哪个容器、以什么身份、执行什么命令”的描述。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerConnection {
    /// 容器名或 ID；名称在运行期解析，重命名后由用户修改会话。
    pub container: String,
    /// 容器内执行用户；`None` 表示镜像默认用户。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// 容器内工作目录；`None` 表示镜像默认工作目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// 启动命令；`None` 表示引擎默认（通常是 `/bin/sh`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// 追加环境变量（`KEY=VALUE`），原样传给 exec，不经过 Shell。
    #[serde(default)]
    pub env: Vec<String>,
    /// 是否申请 TTY；交互式终端固定为 `true`。
    #[serde(default = "default_true")]
    pub tty: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ContainerConnection {
    fn default() -> Self {
        Self {
            container: String::new(),
            user: None,
            working_dir: None,
            command: None,
            env: Vec::new(),
            tty: true,
        }
    }
}

impl ContainerConnection {
    pub fn new(container: impl Into<String>) -> Self {
        Self {
            container: container.into(),
            ..Self::default()
        }
    }

    /// 实际执行的 argv；未配置时回落到镜像的默认 Shell。
    pub fn command_argv(&self) -> Vec<String> {
        self.command
            .clone()
            .filter(|argv| !argv.is_empty())
            .unwrap_or_else(|| vec!["/bin/sh".to_string()])
    }
}

/// 已发现的引擎实例。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEngineInfo {
    /// 已解析为显式端点的引用，UI 原样带回后续命令。
    pub engine: ContainerEngineRef,
    /// 服务端版本，如 `27.3.1`。
    pub version: String,
    /// 协商后的 API 版本，如 `1.46`。
    pub api_version: String,
    /// 服务端平台名，如 `Docker Engine - Community` / `Podman Engine`。
    pub platform: String,
    /// CLI 后端不具备 resize 与部分流式能力。
    pub cli_backend: bool,
}

impl ContainerEngineInfo {
    /// 界面标题：`Docker 27.3.1`。
    pub fn title(&self) -> String {
        format!("{} {}", self.engine.kind.label(), self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_host() {
        assert_eq!(ContainerEndpoint::Auto.browser_host(), "localhost");
        assert_eq!(
            ContainerEndpoint::Unix {
                path: PathBuf::from("/var/run/docker.sock")
            }
            .browser_host(),
            "localhost"
        );
        assert_eq!(
            ContainerEndpoint::Tcp {
                url: "tcp://192.168.1.120:2375".to_string(),
                tls: None,
            }
            .browser_host(),
            "192.168.1.120"
        );
        assert_eq!(
            ContainerEndpoint::Tcp {
                url: "tcp://0.0.0.0:2375".to_string(),
                tls: None,
            }
            .browser_host(),
            "localhost"
        );
        assert_eq!(
            ContainerEndpoint::Ssh {
                host: "remote.server.org".to_string(),
                port: Some(22),
                user: Some("root".to_string()),
                identity_file: None,
                password: None,
                use_sudo: false,
                sudo_password: None,
            }
            .browser_host(),
            "remote.server.org"
        );
    }
}
