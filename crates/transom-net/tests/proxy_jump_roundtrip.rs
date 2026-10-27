//! ProxyJump 集成测试：经过一台跳板机建立到目标主机的 SSH 终端会话。
//!
//! 跳板机服务端实现 `direct-tcpip` 转发（把通道字节桥接到目标监听端口），
//! 目标服务端提供最小交互 Shell。测试验证整条链路可用，且两次握手都走了
//! TOFU 主机密钥记录。

use russh::keys::{Algorithm, PrivateKey, key::safe_rng};
use russh::server::{self, Auth, Msg};
use russh::{Channel, ChannelId};
use std::fmt::Debug;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use transom_core::{
    AuthMethod, ConnectionSecrets, KnownHostsStore, SessionOptions, SshConnection, SshHop,
    TerminalSize,
};
use transom_net::{PasswordPrompter, SshConnectParams, connect_ssh};
use transom_store::JsonKnownHostsStore;

struct EchoServer;

impl server::Server for EchoServer {
    type Handler = EchoServer;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        EchoServer
    }
}

impl server::Handler for EchoServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "tester" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_success(channel);
        session.data(channel, b"TARGET READY\r\n".to_vec())?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, data.to_vec())?;
        Ok(())
    }
}

struct JumpServer;

impl server::Server for JumpServer {
    type Handler = JumpServer;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        JumpServer
    }
}

impl server::Handler for JumpServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "jumper" && password == "jump-secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    /// 跳板机的核心能力：把 `direct-tcpip` 通道桥接到目标地址。
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        let host = host_to_connect.to_string();
        let port = port_to_connect as u16;
        tokio::spawn(async move {
            let Ok(mut tcp) = tokio::net::TcpStream::connect((host.as_str(), port)).await else {
                return;
            };
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        });
        Ok(())
    }
}

fn server_config() -> Arc<server::Config> {
    let mut config = server::Config {
        inactivity_timeout: None,
        ..Default::default()
    };
    config
        .keys
        .push(PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap());
    Arc::new(config)
}

/// 起一个只接受一次连接的 SSH 服务端，返回监听端口与后台任务。
async fn spawn_server<H>(handler: H) -> (u16, tokio::task::JoinHandle<()>)
where
    H: server::Handler + server::Server<Handler = H> + Send + Sync + 'static,
    <H as server::Handler>::Error: Debug,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = server_config();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let handle = server::run_stream(config, socket, handler).await.unwrap();
        let _ = handle.await;
    });
    (port, task)
}

fn ssh_connection(host: &str, port: u16, user: &str) -> SshConnection {
    SshConnection {
        host: host.into(),
        port,
        username: user.into(),
        auth: AuthMethod::Password { save: false },
        options: SessionOptions {
            connect_timeout_secs: 5,
            keepalive_secs: 0,
            allow_keyboard_interactive: false,
            auto_reconnect: false,
            wol_mac: None,
        },
        proxy_jump: None,
    }
}

fn password_secret(value: &str) -> ConnectionSecrets {
    ConnectionSecrets {
        password: Some(zeroize::Zeroizing::new(value.to_string())),
        key_passphrase: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jump_host_tunnels_terminal_session_to_target() {
    let (target_port, target_task) = spawn_server(EchoServer).await;
    let (jump_port, jump_task) = spawn_server(JumpServer).await;

    let dir = tempfile::tempdir().unwrap();
    let known_hosts =
        Arc::new(JsonKnownHostsStore::new(dir.path().join("known_hosts.json")).unwrap());

    let target = ssh_connection("127.0.0.1", target_port, "tester");
    let jumps = vec![SshHop {
        connection: ssh_connection("127.0.0.1", jump_port, "jumper"),
        secrets: password_secret("jump-secret"),
    }];

    let session = tokio::time::timeout(
        Duration::from_secs(10),
        connect_ssh(SshConnectParams {
            connection: &target,
            jumps: &jumps,
            size: TerminalSize::new(80, 24),
            credentials: password_secret("secret"),
            prompter: Arc::new(PasswordPrompter::new("secret")),
            known_hosts: known_hosts.clone(),
            proxy: transom_core::OutboundProxy::default(),
        }),
    )
    .await
    .expect("经跳板机建连超时")
    .expect("经跳板机建连失败");

    // 两跳主机密钥都必须入账：跳板机本身也走 TOFU，而不是被静默跳过。
    let records = known_hosts.list().unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().any(|record| record.port == jump_port));
    assert!(records.iter().any(|record| record.port == target_port));

    let transom_net::SshSession { io, join, .. } = session;
    let control = io.control;
    let mut reader = io.reader;
    let mut writer = io.writer;

    let read_task = tokio::task::spawn_blocking(move || {
        let mut banner = [0u8; 14];
        reader.read_exact(&mut banner)?;
        let mut echo = [0u8; 5];
        reader.read_exact(&mut echo)?;
        Ok::<_, std::io::Error>((banner, echo))
    });
    let write_task = tokio::task::spawn_blocking(move || {
        writer.write_all(b"jump\n")?;
        Ok::<_, std::io::Error>(writer)
    });
    let writer = tokio::time::timeout(Duration::from_secs(5), write_task)
        .await
        .expect("写入超时")
        .unwrap()
        .unwrap();
    let (banner, echo) = tokio::time::timeout(Duration::from_secs(5), read_task)
        .await
        .expect("读取超时")
        .unwrap()
        .unwrap();
    assert_eq!(&banner, b"TARGET READY\r\n");
    assert_eq!(&echo, b"jump\n");

    drop(writer);
    let _ = control.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    jump_task.abort();
    target_task.abort();
}
