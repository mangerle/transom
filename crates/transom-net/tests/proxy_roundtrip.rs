//! 出站代理端到端测试：SOCKS5（带鉴权）与 HTTP CONNECT 隧道后的 SSH 终端。
//!
//! 代理服务端由测试内的最小实现充当：只解析握手、校验鉴权与目标地址，
//! 随后把后续字节透明桥接到目标 SSH 服务。这样验证的是"客户端协议字节
//! 正确、隧道可用"，而不是第三方代理实现。

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use russh::keys::{Algorithm, PrivateKey, key::safe_rng};
use russh::server::{self, Auth, Msg};
use russh::{Channel, ChannelId};
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use transom_core::{
    AuthMethod, ConnectionSecrets, OutboundProxy, ProxyAuth, SessionOptions, SshConnection,
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

async fn spawn_echo_ssh() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = server::Config {
        inactivity_timeout: None,
        ..Default::default()
    };
    config
        .keys
        .push(PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap());
    let config = Arc::new(config);
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let handle = server::run_stream(config, socket, EchoServer)
            .await
            .unwrap();
        let _ = handle.await;
    });
    (port, task)
}

/// 桥接双向字节直到任一端关闭。
async fn bridge(mut left: TcpStream, right: TcpStream) {
    let mut right = right;
    let _ = tokio::io::copy_bidirectional(&mut left, &mut right).await;
}

/// 起一个只接受一次连接的 SOCKS5 代理，校验用户名/密码后桥接到 `expected_target`。
async fn spawn_socks5_proxy(
    expected_target: String,
    username: &'static str,
    password: &'static str,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        // 方法协商：客户端至少提供用户名/密码方式。
        let mut greeting = [0u8; 2];
        client.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting[0], 0x05);
        let mut methods = vec![0u8; greeting[1] as usize];
        client.read_exact(&mut methods).await.unwrap();
        assert!(methods.contains(&0x02), "客户端未提供用户名/密码认证");
        client.write_all(&[0x05, 0x02]).await.unwrap();

        // RFC 1929 鉴权。
        let mut version = [0u8; 1];
        client.read_exact(&mut version).await.unwrap();
        assert_eq!(version[0], 0x01);
        let mut length = [0u8; 1];
        client.read_exact(&mut length).await.unwrap();
        let mut user = vec![0u8; length[0] as usize];
        client.read_exact(&mut user).await.unwrap();
        client.read_exact(&mut length).await.unwrap();
        let mut pass = vec![0u8; length[0] as usize];
        client.read_exact(&mut pass).await.unwrap();
        let accepted = user == username.as_bytes() && pass == password.as_bytes();
        client
            .write_all(&[0x01, if accepted { 0x00 } else { 0x01 }])
            .await
            .unwrap();
        if !accepted {
            return;
        }

        // CONNECT 请求：必须远端解析域名/地址，且目标与预期一致。
        let mut head = [0u8; 4];
        client.read_exact(&mut head).await.unwrap();
        assert_eq!(head[0], 0x05);
        assert_eq!(head[1], 0x01);
        let target = match head[3] {
            0x01 => {
                let mut addr = [0u8; 4];
                client.read_exact(&mut addr).await.unwrap();
                format!(
                    "{}.{}.{}.{}:{}",
                    addr[0],
                    addr[1],
                    addr[2],
                    addr[3],
                    read_port(&mut client).await
                )
            }
            0x03 => {
                let mut length = [0u8; 1];
                client.read_exact(&mut length).await.unwrap();
                let mut host = vec![0u8; length[0] as usize];
                client.read_exact(&mut host).await.unwrap();
                format!(
                    "{}:{}",
                    String::from_utf8_lossy(&host),
                    read_port(&mut client).await
                )
            }
            other => panic!("未预期的地址类型: 0x{other:02x}"),
        };
        assert_eq!(target, expected_target, "CONNECT 目标与预期不一致");
        let upstream = TcpStream::connect(&expected_target).await.unwrap();
        client
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
        bridge(client, upstream).await;
    });
    (port, task)
}

async fn read_port(client: &mut TcpStream) -> u16 {
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await.unwrap();
    u16::from_be_bytes(port)
}

/// 起一个只接受一次连接的 HTTP CONNECT 代理，校验 Basic 鉴权。
async fn spawn_http_proxy(
    expected_target: String,
    username: &'static str,
    password: &'static str,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut header = Vec::with_capacity(256);
        let mut previous = 0u8;
        loop {
            let mut byte = [0u8; 1];
            client.read_exact(&mut byte).await.unwrap();
            header.push(byte[0]);
            if previous == b'\r' && byte[0] == b'\n' {
                let len = header.len();
                if len >= 4 && header[len - 4..] == *b"\r\n\r\n" {
                    break;
                }
            }
            previous = byte[0];
        }
        let text = String::from_utf8_lossy(&header).to_string();
        let request_line = text.lines().next().unwrap_or_default();
        assert_eq!(
            request_line,
            format!("CONNECT {expected_target} HTTP/1.1"),
            "CONNECT 请求行不符合预期"
        );
        let expected_auth = STANDARD.encode(format!("{username}:{password}"));
        assert!(
            text.contains(&format!("Proxy-Authorization: Basic {expected_auth}")),
            "缺少或错误的 Proxy-Authorization"
        );
        let upstream = TcpStream::connect(&expected_target).await.unwrap();
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        bridge(client, upstream).await;
    });
    (port, task)
}

fn ssh_connection(port: u16) -> SshConnection {
    SshConnection {
        host: "127.0.0.1".into(),
        port,
        username: "tester".into(),
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

async fn connect_through(proxy: OutboundProxy, target_port: u16) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let known_hosts =
        Arc::new(JsonKnownHostsStore::new(dir.path().join("known_hosts.json")).unwrap());
    let connection = ssh_connection(target_port);
    let session = tokio::time::timeout(
        Duration::from_secs(10),
        connect_ssh(SshConnectParams {
            connection: &connection,
            jumps: &[],
            size: TerminalSize::new(80, 24),
            credentials: ConnectionSecrets {
                password: Some(zeroize::Zeroizing::new("secret".to_string())),
                key_passphrase: None,
            },
            prompter: Arc::new(PasswordPrompter::new("secret")),
            known_hosts,
            proxy,
            allow_sha1_publickey_auth: true,
        }),
    )
    .await
    .map_err(|_| "建连超时".to_string())?
    .map_err(|error| error.to_string())?;

    let transom_net::SshSession { io, join, .. } = session;
    let control = io.control;
    let mut reader = io.reader;
    let mut writer = io.writer;
    let read_task = tokio::task::spawn_blocking(move || {
        let mut banner = [0u8; 14];
        reader.read_exact(&mut banner)?;
        let mut echo = [0u8; 6];
        reader.read_exact(&mut echo)?;
        Ok::<_, std::io::Error>((banner, echo))
    });
    let write_task = tokio::task::spawn_blocking(move || {
        writer.write_all(b"proxy\n")?;
        Ok::<_, std::io::Error>(writer)
    });
    let writer = tokio::time::timeout(Duration::from_secs(5), write_task)
        .await
        .map_err(|_| "写入超时".to_string())?
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let (banner, echo) = tokio::time::timeout(Duration::from_secs(5), read_task)
        .await
        .map_err(|_| "读取超时".to_string())?
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    assert_eq!(&banner, b"TARGET READY\r\n");
    assert_eq!(&echo, b"proxy\n");
    drop(writer);
    let _ = control.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ssh_through_socks5_with_password_auth() {
    let (target_port, target_task) = spawn_echo_ssh().await;
    let (proxy_port, proxy_task) = spawn_socks5_proxy(
        format!("127.0.0.1:{target_port}"),
        "proxy-user",
        "proxy-pass",
    )
    .await;
    let proxy = OutboundProxy::Socks5 {
        host: "127.0.0.1".into(),
        port: proxy_port,
        auth: Some(ProxyAuth {
            username: "proxy-user".into(),
            password: "proxy-pass".into(),
        }),
    };
    connect_through(proxy, target_port).await.unwrap();
    proxy_task.abort();
    target_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ssh_through_http_connect_with_basic_auth() {
    let (target_port, target_task) = spawn_echo_ssh().await;
    let (proxy_port, proxy_task) = spawn_http_proxy(
        format!("127.0.0.1:{target_port}"),
        "proxy-user",
        "proxy-pass",
    )
    .await;
    let proxy = OutboundProxy::Http {
        host: "127.0.0.1".into(),
        port: proxy_port,
        auth: Some(ProxyAuth {
            username: "proxy-user".into(),
            password: "proxy-pass".into(),
        }),
    };
    connect_through(proxy, target_port).await.unwrap();
    proxy_task.abort();
    target_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn socks5_rejects_wrong_password() {
    let (target_port, target_task) = spawn_echo_ssh().await;
    let (proxy_port, proxy_task) =
        spawn_socks5_proxy(format!("127.0.0.1:{target_port}"), "proxy-user", "right").await;
    let proxy = OutboundProxy::Socks5 {
        host: "127.0.0.1".into(),
        port: proxy_port,
        auth: Some(ProxyAuth {
            username: "proxy-user".into(),
            password: "wrong".into(),
        }),
    };
    let error = connect_through(proxy, target_port).await.unwrap_err();
    assert!(error.contains("代理"), "错误应指明代理环节: {error}");
    assert!(error.contains("密码"), "错误应指明鉴权失败: {error}");
    proxy_task.abort();
    target_task.abort();
}
