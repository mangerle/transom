//! 引擎通道：把不同平台的「连上守护进程」方式收敛为一条双向字节流。
//!
//! 设计原理（Why）：Docker/Podman 的协议是 HTTP/1.1，真正随平台变化的只有
//! 底层通道（Unix socket、Windows 命名管道、TCP、子进程 stdio）。把通道
//! 抽象成一个 [`EngineStream`] 后，HTTP 客户端、exec Upgrade 与日志流
//! 只需实现一次，新增平台通道不影响协议代码。

use crate::error::ContainerError;
mod ssh_stdio;
use ssh_stdio::SshConnectSpec;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// 单次连接建立的超时：守护进程未启动时快速失败，避免界面长时间转圈。
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 引擎通道的公共约束。
pub trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

/// 一条已建立、可读写的引擎通道。
pub type EngineStream = Box<dyn AsyncStream>;

/// 平台相关的通道描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineTransport {
    /// Unix 域套接字（Linux/macOS）。
    Unix { path: PathBuf },
    /// Windows 命名管道。
    NamedPipe { path: String },
    /// TCP 端点（`tcp://host:port`），支持可选 TLS。
    Tcp {
        url: String,
        tls: Option<transom_core::TlsConfig>,
    },
    /// 子进程 stdio：WSL 的 `docker system dial-stdio` 等场景复用同一条
    /// HTTP 客户端，不为此再写一套 CLI 解析。
    Stdio { program: String, args: Vec<String> },
    /// 原生 SSH 隧道：通过内置 russh 客户端直接打开通道执行远端命令，
    /// 完整支持账号密码、私钥与 sudo -S 提权注入。
    Ssh {
        host: String,
        port: u16,
        user: String,
        password: Option<String>,
        identity_file: Option<String>,
        use_sudo: bool,
        sudo_password: Option<String>,
        command: String,
    },
}

impl EngineTransport {
    /// 日志与界面摘要。
    pub fn describe(&self) -> String {
        match self {
            Self::Unix { path } => format!("unix://{}", path.display()),
            Self::NamedPipe { path } => format!("npipe:{path}"),
            Self::Tcp { url, tls } => {
                if tls.is_some() {
                    format!("{url} (TLS)")
                } else {
                    url.clone()
                }
            }
            Self::Stdio { program, args } => {
                if args.is_empty() {
                    format!("stdio://{program}")
                } else {
                    format!("stdio://{program} {}", args.join(" "))
                }
            }
            Self::Ssh {
                host,
                port,
                user,
                use_sudo,
                ..
            } => {
                let sudo_str = if *use_sudo { " (sudo)" } else { "" };
                format!("ssh://{user}@{host}:{port}{sudo_str}")
            }
        }
    }

    /// 建立通道；超时与连接错误都带端点上下文。
    ///
    /// # Errors
    /// - 端点不存在、权限不足或对端未监听时返回 [`ContainerError::Connect`]；
    /// - 当前平台不支持该通道类型时返回 [`ContainerError::Unsupported`]。
    pub async fn connect(&self) -> Result<EngineStream, ContainerError> {
        match self {
            Self::Unix { path } => self.connect_unix(path).await,
            Self::NamedPipe { path } => self.connect_named_pipe(path).await,
            Self::Tcp { url, tls } => self.connect_tcp(url, tls.as_ref()).await,
            Self::Stdio { program, args } => self.connect_stdio(program, args),
            Self::Ssh {
                host,
                port,
                user,
                password,
                identity_file,
                use_sudo,
                sudo_password,
                command,
            } => {
                self.connect_ssh(SshConnectSpec {
                    host,
                    port: *port,
                    user,
                    password: password.as_deref(),
                    identity_file: identity_file.as_deref(),
                    use_sudo: *use_sudo,
                    sudo_password: sudo_password.as_deref(),
                    command,
                })
                .await
            }
        }
    }

    /// 通过 Unix Domain Socket 连接容器引擎。
    async fn connect_unix(&self, path: &Path) -> Result<EngineStream, ContainerError> {
        #[cfg(unix)]
        {
            let socket =
                tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::UnixStream::connect(path))
                    .await
                    .map_err(|_| ContainerError::Connect {
                        endpoint: self.describe(),
                        source: std::io::Error::new(std::io::ErrorKind::TimedOut, "连接超时"),
                    })?
                    .map_err(|source| ContainerError::Connect {
                        endpoint: self.describe(),
                        source,
                    })?;
            Ok(Box::new(socket))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(ContainerError::Unsupported {
                operation: "当前平台不支持 Unix socket".to_string(),
            })
        }
    }

    /// 通过 Windows 命名管道连接容器引擎。
    async fn connect_named_pipe(&self, path: &str) -> Result<EngineStream, ContainerError> {
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;
            let pipe =
                tokio::time::timeout(CONNECT_TIMEOUT, async { ClientOptions::new().open(path) })
                    .await
                    .map_err(|_| ContainerError::Connect {
                        endpoint: self.describe(),
                        source: std::io::Error::new(std::io::ErrorKind::TimedOut, "连接超时"),
                    })?
                    .map_err(|source| ContainerError::Connect {
                        endpoint: self.describe(),
                        source,
                    })?;
            Ok(Box::new(pipe))
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(ContainerError::Unsupported {
                operation: "当前平台不支持命名管道".to_string(),
            })
        }
    }

    /// 通过 TCP（可选 TLS）连接容器引擎。
    async fn connect_tcp(
        &self,
        url: &str,
        tls: Option<&transom_core::TlsConfig>,
    ) -> Result<EngineStream, ContainerError> {
        let address = strip_tcp_scheme(url);
        let socket =
            tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(&address))
                .await
                .map_err(|_| ContainerError::Connect {
                    endpoint: self.describe(),
                    source: std::io::Error::new(std::io::ErrorKind::TimedOut, "连接超时"),
                })?
                .map_err(|source| ContainerError::Connect {
                    endpoint: self.describe(),
                    source,
                })?;
        socket.set_nodelay(true).ok();

        if let Some(tls_cfg) = tls {
            let host = extract_host(&address);
            let tls_stream = crate::tls::connect_tls(socket, &host, tls_cfg).await?;
            Ok(Box::new(tls_stream))
        } else {
            Ok(Box::new(socket))
        }
    }
}

impl fmt::Display for EngineTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// 去掉 `tcp://`、`tls://` 等前缀，保留 `host:port`。
fn strip_tcp_scheme(url: &str) -> String {
    url.strip_prefix("tcp://")
        .or_else(|| url.strip_prefix("tls://"))
        .or_else(|| url.strip_prefix("https://"))
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .to_string()
}

/// 从 `host:port` 或 IPv6 中提取纯主机名或 IP。
fn extract_host(address: &str) -> String {
    if let Some((host, _)) = address.rsplit_once(':') {
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .to_string()
    } else {
        address.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_host, strip_tcp_scheme};

    #[test]
    fn tcp_scheme_is_stripped_for_tokio_connect() {
        assert_eq!(strip_tcp_scheme("tcp://127.0.0.1:2375"), "127.0.0.1:2375");
        assert_eq!(strip_tcp_scheme("tls://127.0.0.1:2376"), "127.0.0.1:2376");
        assert_eq!(
            strip_tcp_scheme("https://my-docker.local:2376"),
            "my-docker.local:2376"
        );
        assert_eq!(strip_tcp_scheme("127.0.0.1:2375"), "127.0.0.1:2375");
    }

    #[test]
    fn host_is_extracted_from_address() {
        assert_eq!(extract_host("127.0.0.1:2376"), "127.0.0.1");
        assert_eq!(extract_host("docker.lan:2376"), "docker.lan");
        assert_eq!(extract_host("[::1]:2376"), "::1");
        assert_eq!(extract_host("docker.lan"), "docker.lan");
    }
}
