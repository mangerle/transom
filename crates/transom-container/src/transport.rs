//! 引擎通道：把不同平台的「连上守护进程」方式收敛为一条双向字节流。
//!
//! 设计原理（Why）：Docker/Podman 的协议是 HTTP/1.1，真正随平台变化的只有
//! 底层通道（Unix socket、Windows 命名管道、TCP、子进程 stdio）。把通道
//! 抽象成一个 [`EngineStream`] 后，HTTP 客户端、exec Upgrade 与日志流
//! 只需实现一次，新增平台通道不影响协议代码。

use crate::error::ContainerError;
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use std::fmt;
use std::path::PathBuf;
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
            Self::Unix { path } => {
                #[cfg(unix)]
                {
                    let socket = tokio::time::timeout(
                        CONNECT_TIMEOUT,
                        tokio::net::UnixStream::connect(path),
                    )
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
            Self::NamedPipe { path } => {
                #[cfg(windows)]
                {
                    use tokio::net::windows::named_pipe::ClientOptions;
                    let pipe = tokio::time::timeout(CONNECT_TIMEOUT, async {
                        ClientOptions::new().open(path)
                    })
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
            Self::Tcp { url, tls } => {
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

                if let Some(tls_cfg) = tls.as_ref() {
                    let host = extract_host(&address);
                    let tls_stream = crate::tls::connect_tls(socket, &host, tls_cfg).await?;
                    Ok(Box::new(tls_stream))
                } else {
                    Ok(Box::new(socket))
                }
            }
            Self::Stdio { program, args } => {
                let mut command = tokio::process::Command::new(program);
                command
                    .args(args)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut child = command.group().kill_on_drop(true).spawn().map_err(|error| {
                    let friendly_msg = if error.kind() == std::io::ErrorKind::NotFound {
                        if program == "ssh" {
                            "未检测到系统的 ssh 命令，请安装 OpenSSH 客户端并确保已配置在 PATH 环境变量中".to_string()
                        } else if program.contains("wsl") {
                            "未检测到 wsl.exe 命令，请确保宿主机已启用 WSL 子系统".to_string()
                        } else {
                            format!("可执行命令未找到: {program}")
                        }
                    } else {
                        error.to_string()
                    };
                    ContainerError::Connect {
                        endpoint: self.describe(),
                        source: std::io::Error::new(error.kind(), friendly_msg),
                    }
                })?;
                let stdin = child
                    .inner()
                    .stdin
                    .take()
                    .ok_or_else(|| ContainerError::Connect {
                        endpoint: self.describe(),
                        source: std::io::Error::other("子进程标准输入不可用"),
                    })?;
                let stdout =
                    child
                        .inner()
                        .stdout
                        .take()
                        .ok_or_else(|| ContainerError::Connect {
                            endpoint: self.describe(),
                            source: std::io::Error::other("子进程标准输出不可用"),
                        })?;
                // stderr 只用于诊断：后台读取并写入 debug 日志，避免 pipe 写满
                // 反过来阻塞通道进程。
                if let Some(stderr) = child.inner().stderr.take() {
                    let endpoint = self.describe();
                    tokio::spawn(async move {
                        use tokio::io::AsyncBufReadExt;
                        let reader = tokio::io::BufReader::new(stderr);
                        let mut lines = reader.lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            log::debug!("容器引擎通道 stderr，端点: {endpoint}，输出: {line}");
                        }
                    });
                }
                Ok(Box::new(ChildStream {
                    _child: child,
                    stdin,
                    stdout,
                }))
            }
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
                let mut connection = transom_core::SshConnection::new(host.clone(), user.clone());
                connection.port = *port;
                if let Some(path) = identity_file {
                    connection.auth = transom_core::AuthMethod::PrivateKey {
                        path: PathBuf::from(path),
                        save_passphrase: false,
                        cert_path: None,
                    };
                } else if password.is_some() {
                    connection.auth = transom_core::AuthMethod::Password { save: false };
                } else {
                    connection.auth = transom_core::AuthMethod::Agent;
                }

                let mut credentials = transom_core::ConnectionSecrets::default();
                if let Some(pwd) = password {
                    credentials.password = Some(zeroize::Zeroizing::new(pwd.clone()));
                }

                let prompter = if let Some(pwd) = password {
                    std::sync::Arc::new(transom_net::PasswordPrompter::new(pwd.clone()))
                } else {
                    std::sync::Arc::new(transom_net::PasswordPrompter::new(""))
                };
                let known_hosts = std::sync::Arc::new(transom_net::AutoTrustKnownHosts::new());

                let params = transom_net::SshRemoteConnectParams {
                    connection: &connection,
                    jumps: &[],
                    credentials,
                    prompter,
                    known_hosts,
                    proxy: transom_core::OutboundProxy::default(),
                    allow_sha1_publickey_auth: true,
                };

                let remote = tokio::time::timeout(
                    Duration::from_secs(10),
                    transom_net::connect_ssh_remote(params),
                )
                .await
                .map_err(|_| ContainerError::Connect {
                    endpoint: self.describe(),
                    source: std::io::Error::new(std::io::ErrorKind::TimedOut, "SSH 连接超时"),
                })?
                .map_err(|err| ContainerError::Connect {
                    endpoint: self.describe(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        err.to_string(),
                    ),
                })?;

                let full_command = if *use_sudo {
                    format!("sudo -S -p '' {command}")
                } else {
                    command.clone()
                };

                let mut channel_stream =
                    remote
                        .open_exec_stream(&full_command)
                        .await
                        .map_err(|err| ContainerError::Connect {
                            endpoint: self.describe(),
                            source: std::io::Error::new(
                                std::io::ErrorKind::ConnectionAborted,
                                err.to_string(),
                            ),
                        })?;

                if *use_sudo {
                    use tokio::io::AsyncWriteExt;
                    let effective_pwd = sudo_password
                        .as_deref()
                        .or(password.as_deref())
                        .unwrap_or_default();
                    channel_stream
                        .write_all(format!("{effective_pwd}\n").as_bytes())
                        .await
                        .map_err(|err| ContainerError::Connect {
                            endpoint: self.describe(),
                            source: std::io::Error::new(
                                std::io::ErrorKind::PermissionDenied,
                                format!("向 sudo 注入密码失败: {err}"),
                            ),
                        })?;
                    channel_stream
                        .flush()
                        .await
                        .map_err(|err| ContainerError::Connect {
                            endpoint: self.describe(),
                            source: std::io::Error::new(
                                std::io::ErrorKind::PermissionDenied,
                                format!("刷新 sudo 密码缓冲区失败: {err}"),
                            ),
                        })?;
                }

                Ok(Box::new(SshEngineStream {
                    stream: channel_stream,
                    _remote: remote,
                }))
            }
        }
    }
}

/// 原生 SSH 命令执行流包装。
struct SshEngineStream {
    stream: transom_net::AsyncChannelStream,
    _remote: transom_net::SshRemote,
}

impl AsyncRead for SshEngineStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for SshEngineStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stream).poll_shutdown(cx)
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

/// 子进程 stdio 的双向流。
///
/// `_child` 必须与流一起存活：提前 drop 会关闭管道；设置了 `kill_on_drop`
/// 后，通道被回收时子进程也会被终止，不留孤儿进程。
struct ChildStream {
    _child: AsyncGroupChild,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
}

impl Drop for ChildStream {
    fn drop(&mut self) {
        // 流被回收时主动终止整个进程组；Tokio 随后会完成子进程状态回收。
        let _ = self._child.start_kill();
    }
}

impl AsyncRead for ChildStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stdout).poll_read(cx, buf)
    }
}

impl AsyncWrite for ChildStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stdin).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stdin).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.stdin).poll_shutdown(cx)
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
