//! 引擎通道：把不同平台的「连上守护进程」方式收敛为一条双向字节流。
//!
//! 设计原理（Why）：Docker/Podman 的协议是 HTTP/1.1，真正随平台变化的只有
//! 底层通道（Unix socket、Windows 命名管道、TCP、子进程 stdio）。把通道
//! 抽象成一个 [`EngineStream`] 后，HTTP 客户端、exec Upgrade 与日志流
//! 只需实现一次，新增平台通道不影响协议代码。

use crate::error::ContainerError;
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
}

impl EngineTransport {
    /// 日志与界面摘要。
    pub fn describe(&self) -> String {
        match self {
            Self::Unix { path } => format!("unix://{}", path.display()),
            Self::NamedPipe { path } => format!("npipe:{path}"),
            Self::Tcp { url, tls } => {
                if tls.as_ref().is_some_and(|c| !c.is_empty()) {
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

                if let Some(tls_cfg) = tls.as_ref().filter(|c| !c.is_empty()) {
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
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);
                let mut child = command.spawn().map_err(|error| ContainerError::Connect {
                    endpoint: self.describe(),
                    source: error,
                })?;
                let stdin = child.stdin.take().ok_or_else(|| ContainerError::Connect {
                    endpoint: self.describe(),
                    source: std::io::Error::other("子进程标准输入不可用"),
                })?;
                let stdout = child.stdout.take().ok_or_else(|| ContainerError::Connect {
                    endpoint: self.describe(),
                    source: std::io::Error::other("子进程标准输出不可用"),
                })?;
                // stderr 只用于诊断：后台读取并写入 debug 日志，避免 pipe 写满
                // 反过来阻塞通道进程。
                if let Some(stderr) = child.stderr.take() {
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

/// 子进程 stdio 的双向流。
///
/// `_child` 必须与流一起存活：提前 drop 会关闭管道；设置了 `kill_on_drop`
/// 后，通道被回收时子进程也会被终止，不留孤儿进程。
struct ChildStream {
    _child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
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
