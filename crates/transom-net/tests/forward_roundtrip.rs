use russh::keys::{Algorithm, PrivateKey, key::safe_rng};
use russh::server::{self, Auth, Msg, Server as _};
use russh::{Channel, ChannelId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use transom_core::{
    AuthMethod, BindAddress, ConnectionSecrets, ForwardKind, ForwardRule, SessionOptions,
    SocksOptions, SshConnection, TargetAddress, TerminalSize,
};
use transom_net::{PasswordPrompter, SshSession, connect_ssh, start_forward};
use transom_store::JsonKnownHostsStore;

struct TestServer;

impl server::Server for TestServer {
    type Handler = TestServerHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        TestServerHandler {
            channels: HashMap::new(),
        }
    }
}

struct TestServerHandler {
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for TestServerHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        tokio::spawn(async move {
            let stream = channel.into_stream();
            let (mut reader, mut writer) = tokio::io::split(stream);
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(count) => {
                        if writer.write_all(&buffer[..count]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        port: &mut u32,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(_) => return Ok(false),
        };
        let actual_port = listener.local_addr().unwrap().port();
        *port = actual_port as u32;
        let handle = session.handle();
        tokio::spawn(async move {
            while let Ok((mut tcp, peer)) = listener.accept().await {
                let handle = handle.clone();
                tokio::spawn(async move {
                    if let Ok(channel) = handle
                        .channel_open_forwarded_tcpip(
                            "127.0.0.1",
                            actual_port as u32,
                            peer.ip().to_string(),
                            peer.port() as u32,
                        )
                        .await
                    {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
                    }
                });
            }
        });
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        _address: &str,
        _port: u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

async fn connect(port: u16, dir: &tempfile::TempDir) -> SshSession {
    let known_hosts =
        Arc::new(JsonKnownHostsStore::new(dir.path().join("known_hosts.json")).unwrap());
    let connection = SshConnection {
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
    };
    tokio::time::timeout(
        Duration::from_secs(8),
        connect_ssh(transom_net::SshConnectParams {
            connection: &connection,
            jumps: &[],
            size: TerminalSize::new(80, 24),
            credentials: ConnectionSecrets {
                password: Some(zeroize::Zeroizing::new("secret".to_string())),
                key_passphrase: None,
            },
            prompter: Arc::new(PasswordPrompter::new("secret")),
            known_hosts,
            proxy: transom_core::OutboundProxy::default(),
        }),
    )
    .await
    .expect("connect timed out")
    .expect("connect failed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_and_dynamic_forwarding_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let mut server_config = server::Config {
        inactivity_timeout: None,
        ..Default::default()
    };
    server_config
        .keys
        .push(PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap());
    let server_config = Arc::new(server_config);
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let handle = server::run_stream(server_config, socket, TestServer.new_client(None))
            .await
            .unwrap();
        let _ = handle.await;
    });

    let session = connect(port, &dir).await;
    let SshSession {
        io, join, remote, ..
    } = session;

    let local_rule = ForwardRule::new(
        "local",
        ForwardKind::Local {
            bind: BindAddress {
                host: "127.0.0.1".into(),
                port: 0,
            },
            target: TargetAddress {
                host: "example.com".into(),
                port: 80,
            },
        },
    );
    let local_task = start_forward(&remote, &local_rule)
        .await
        .expect("start local forward");
    let local_port = local_task.bound_port.expect("local port");
    let mut stream = TcpStream::connect(("127.0.0.1", local_port))
        .await
        .expect("connect local forward");
    stream.write_all(b"hello").await.unwrap();
    let mut buffer = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(3), stream.read_exact(&mut buffer))
        .await
        .expect("local echo timed out")
        .unwrap();
    assert_eq!(&buffer, b"hello");
    local_task.stop().await;

    let dynamic_rule = ForwardRule::new(
        "dynamic",
        ForwardKind::Dynamic {
            bind: BindAddress {
                host: "127.0.0.1".into(),
                port: 0,
            },
            socks: SocksOptions::default(),
        },
    );
    let dynamic_task = start_forward(&remote, &dynamic_rule)
        .await
        .expect("start dynamic forward");
    let dynamic_port = dynamic_task.bound_port.expect("dynamic port");
    let mut stream = TcpStream::connect(("127.0.0.1", dynamic_port))
        .await
        .expect("connect socks");
    stream.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0u8; 2];
    stream.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);

    let domain = b"example.com";
    let mut request = vec![5, 1, 0, 3, domain.len() as u8];
    request.extend_from_slice(domain);
    request.extend_from_slice(&80u16.to_be_bytes());
    stream.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[0], 5);
    assert_eq!(reply[1], 0);

    stream.write_all(b"socks").await.unwrap();
    let mut buffer = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(3), stream.read_exact(&mut buffer))
        .await
        .expect("socks echo timed out")
        .unwrap();
    assert_eq!(&buffer, b"socks");

    // 测试 SOCKS5 UDP ASSOCIATE 功能
    let target_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target_udp_port = target_udp.local_addr().unwrap().port();
    let udp_echo_task = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        if let Ok((len, peer)) = target_udp.recv_from(&mut buf).await {
            let _ = target_udp.send_to(&buf[..len], peer).await;
        }
    });

    let mut udp_control_stream = TcpStream::connect(("127.0.0.1", dynamic_port))
        .await
        .expect("connect socks for udp associate");
    udp_control_stream.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0u8; 2];
    udp_control_stream.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);

    // 请求 UDP ASSOCIATE (CMD = 3)
    let request = [5, 3, 0, 1, 0, 0, 0, 0, 0, 0];
    udp_control_stream.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    udp_control_stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[0], 5);
    assert_eq!(reply[1], 0); // 成功响应
    let bnd_port = u16::from_be_bytes([reply[8], reply[9]]);
    assert!(bnd_port > 0);

    // 客户端通过 UDP socket 向 SOCKS5 relay 端口发送数据
    let client_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut packet = vec![0, 0, 0, 1, 127, 0, 0, 1];
    packet.extend_from_slice(&target_udp_port.to_be_bytes());
    packet.extend_from_slice(b"udp-socks");
    client_udp
        .send_to(&packet, ("127.0.0.1", bnd_port))
        .await
        .unwrap();

    let mut response_buf = [0u8; 1024];
    let (resp_len, _) = tokio::time::timeout(
        Duration::from_secs(3),
        client_udp.recv_from(&mut response_buf),
    )
    .await
    .expect("udp socks echo timed out")
    .unwrap();
    assert!(resp_len >= 10);
    assert_eq!(&response_buf[10..resp_len], b"udp-socks");

    drop(udp_control_stream);
    udp_echo_task.abort();

    dynamic_task.stop().await;

    let echo_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo_listener.local_addr().unwrap().port();
    let echo_task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = echo_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) => break,
                        Ok(count) => {
                            if stream.write_all(&buffer[..count]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    let remote_rule = ForwardRule::new(
        "remote",
        ForwardKind::Remote {
            bind: BindAddress {
                host: "127.0.0.1".into(),
                port: 0,
            },
            target: TargetAddress {
                host: "127.0.0.1".into(),
                port: echo_port,
            },
        },
    );
    let remote_task = start_forward(&remote, &remote_rule)
        .await
        .expect("start remote forward");
    let remote_port = remote_task.bound_port.expect("remote port");
    let mut stream = TcpStream::connect(("127.0.0.1", remote_port))
        .await
        .expect("connect remote forward");
    stream.write_all(b"remote").await.unwrap();
    let mut buffer = [0u8; 6];
    tokio::time::timeout(Duration::from_secs(3), stream.read_exact(&mut buffer))
        .await
        .expect("remote echo timed out")
        .unwrap();
    assert_eq!(&buffer, b"remote");
    remote_task.stop().await;
    echo_task.abort();

    drop(io);
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    server_task.abort();
}
