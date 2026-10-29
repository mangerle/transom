use russh::keys::{Algorithm, PrivateKey, key::safe_rng};
use russh::server::{self, Auth, Msg};
use russh::{Channel, ChannelId};
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use transom_core::{
    AuthMethod, ConnectionSecrets, HostKeyStatus, KnownHostsStore, SessionOptions, SshConnection,
    TerminalSize,
};
use transom_net::{PasswordPrompter, connect_ssh};
use transom_store::JsonKnownHostsStore;

struct TestServer;

impl server::Server for TestServer {
    type Handler = TestServer;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        TestServer
    }
}

impl server::Handler for TestServer {
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
        session.data(channel, b"READY\r\n".to_vec())?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn password_auth_pty_and_tofu_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let known_hosts =
        Arc::new(JsonKnownHostsStore::new(dir.path().join("known_hosts.json")).unwrap());

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
        let handle = server::run_stream(server_config, socket, TestServer)
            .await
            .unwrap();
        let _ = handle.await;
    });

    let connection = SshConnection {
        host: "127.0.0.1".into(),
        port,
        username: "tester".into(),
        auth: AuthMethod::Password { save: false },
        options: SessionOptions {
            connect_timeout_secs: 5,
            keepalive_secs: 0,
            allow_keyboard_interactive: true,

            auto_reconnect: false,
            agent_forward: false,
            wol_mac: None,
        },
        proxy_jump: None,
    };

    let session = tokio::time::timeout(
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
            known_hosts: known_hosts.clone(),
            proxy: transom_core::OutboundProxy::default(),
            allow_sha1_publickey_auth: true,
        }),
    )
    .await
    .expect("connect timed out")
    .expect("connect failed");

    let records = known_hosts.list().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].host, "127.0.0.1");
    assert_eq!(records[0].port, port);
    assert!(!records[0].fingerprint.is_empty());
    assert_eq!(
        known_hosts
            .check(
                "127.0.0.1",
                port,
                &transom_core::HostKeyInfo::new(
                    records[0].algorithm.clone(),
                    records[0].fingerprint.clone(),
                    records[0].public_key.clone(),
                ),
            )
            .unwrap(),
        HostKeyStatus::Match
    );

    let transom_net::SshSession { io, join, .. } = session;
    let control = io.control;
    let mut reader = io.reader;
    let mut writer = io.writer;

    let read_task = tokio::task::spawn_blocking(move || {
        let mut banner = [0u8; 7];
        reader.read_exact(&mut banner)?;
        let mut echo = [0u8; 6];
        reader.read_exact(&mut echo)?;
        Ok::<_, std::io::Error>((banner, echo))
    });

    let write_task = tokio::task::spawn_blocking(move || {
        writer.write_all(b"hello\n")?;
        Ok::<_, std::io::Error>(writer)
    });
    let writer = tokio::time::timeout(Duration::from_secs(5), write_task)
        .await
        .expect("write timed out")
        .unwrap()
        .unwrap();
    let (banner, echo) = tokio::time::timeout(Duration::from_secs(5), read_task)
        .await
        .expect("read timed out")
        .unwrap()
        .unwrap();
    assert_eq!(&banner, b"READY\r\n");
    assert_eq!(&echo, b"hello\n");

    drop(writer);
    let _ = control.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    server_task.abort();
}
