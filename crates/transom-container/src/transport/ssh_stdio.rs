//! 子进程 stdio 与原生 SSH 远程容器引擎通道实现。

use super::{EngineStream, EngineTransport};
use crate::error::ContainerError;
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

impl EngineTransport {
    /// 启动子进程通过标准输入输出流隧道连接容器引擎。
    pub(super) fn connect_stdio(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<EngineStream, ContainerError> {
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.group().kill_on_drop(true).spawn().map_err(|error| {
            let friendly_msg = if error.kind() == std::io::ErrorKind::NotFound {
                if program == "ssh" {
                    "未检测到系统的 ssh 命令，请安装 OpenSSH 客户端并确保已配置在 PATH 环境变量中"
                        .to_string()
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
        let stdout = child
            .inner()
            .stdout
            .take()
            .ok_or_else(|| ContainerError::Connect {
                endpoint: self.describe(),
                source: std::io::Error::other("子进程标准输出不可用"),
            })?;
        // stderr 只用于诊断：后台读取并写入 debug 日志，避免 pipe 写满
        // 反过来阻塞通道进程；句柄随 ChildStream 持有并在析构时清理。
        let stderr_task = child.inner().stderr.take().map(|stderr| {
            let endpoint = self.describe();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("容器引擎通道 stderr，端点: {endpoint}，输出: {line}");
                }
            })
        });
        Ok(Box::new(ChildStream {
            _child: child,
            stderr_task,
            stdin,
            stdout,
        }))
    }

    /// 通过内置 SSH 通道建立远程容器引擎执行流。
    pub(super) async fn connect_ssh(
        &self,
        spec: SshConnectSpec<'_>,
    ) -> Result<EngineStream, ContainerError> {
        let mut connection =
            transom_core::SshConnection::new(spec.host.to_string(), spec.user.to_string());
        connection.port = spec.port;
        if let Some(path) = spec.identity_file {
            connection.auth = transom_core::AuthMethod::PrivateKey {
                path: PathBuf::from(path),
                save_passphrase: false,
                cert_path: None,
            };
        } else if spec.password.is_some() {
            connection.auth = transom_core::AuthMethod::Password { save: false };
        } else {
            connection.auth = transom_core::AuthMethod::Agent;
        }

        let mut credentials = transom_core::ConnectionSecrets::default();
        if let Some(pwd) = spec.password {
            credentials.password = Some(zeroize::Zeroizing::new(pwd.to_string()));
        }

        let prompter = std::sync::Arc::new(transom_net::PasswordPrompter::new(
            spec.password.unwrap_or_default(),
        ));
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
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, err.to_string()),
        })?;

        let full_command = if spec.use_sudo {
            format!("sudo -S -p '' {}", spec.command)
        } else {
            spec.command.to_string()
        };

        let mut channel_stream = remote
            .open_exec_stream(&full_command)
            .await
            .map_err(|err| ContainerError::Connect {
                endpoint: self.describe(),
                source: std::io::Error::new(std::io::ErrorKind::ConnectionAborted, err.to_string()),
            })?;

        if spec.use_sudo {
            use tokio::io::AsyncWriteExt;
            let effective_pwd = spec.sudo_password.or(spec.password).unwrap_or_default();
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

/// SSH 通道连接参数对象，避免多参数平铺。
pub(super) struct SshConnectSpec<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) user: &'a str,
    pub(super) password: Option<&'a str>,
    pub(super) identity_file: Option<&'a str>,
    pub(super) use_sudo: bool,
    pub(super) sudo_password: Option<&'a str>,
    pub(super) command: &'a str,
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

/// 子进程 stdio 的双向流。
///
/// `_child` 必须与流一起存活：提前 drop 会关闭管道；设置了 `kill_on_drop`
/// 后，通道被回收时子进程也会被终止，不留孤儿进程。
struct ChildStream {
    _child: AsyncGroupChild,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
}

impl Drop for ChildStream {
    fn drop(&mut self) {
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
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
