//! CA 签发的 OpenSSH 用户证书认证：证书被接受、以及证书被拒时回退裸公钥两条路径。
//!
//! 测试服务端不校验 CA 签名（russh 的 `auth_openssh_certificate` 回调交给我们自己判定），
//! 因此这里验证的是**客户端行为**：配了证书就必须把证书发出去、成功时不多试一次公钥、
//! 被拒时才回退。CA 信任链的校验属于服务器/运维侧策略，不在本仓库职责内。

use parking_lot::Mutex;
use russh::keys::key::safe_rng;
use russh::keys::{Algorithm, Certificate, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg};
use russh::{Channel, ChannelId};
use ssh_key::LineEnding;
use ssh_key::certificate::{Builder, CertType};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use transom_core::{AuthMethod, ConnectionSecrets, SessionOptions, SshConnection, TerminalSize};
use transom_net::{PasswordPrompter, connect_ssh};
use transom_store::JsonKnownHostsStore;

/// 记录服务端看到的认证尝试，顺序即客户端的尝试次序。
#[derive(Clone)]
struct Observed(Arc<Mutex<Vec<String>>>);

impl Observed {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    fn record(&self, entry: &str) {
        self.0.lock().push(entry.to_string());
    }

    fn events(&self) -> Vec<String> {
        self.0.lock().clone()
    }
}

struct CertServer {
    observed: Observed,
}

impl server::Server for CertServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        Self {
            observed: self.observed.clone(),
        }
    }
}

impl server::Handler for CertServer {
    type Error = russh::Error;

    /// `certuser` 代表信任该 CA 的服务器：只接受证书。
    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        _certificate: &Certificate,
    ) -> Result<Auth, Self::Error> {
        self.observed.record(&format!("cert:{user}"));
        Ok(if user == "certuser" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    /// `keyuser` 代表没配 CA 但仍认这把私钥的服务器：只接受裸公钥。
    async fn auth_publickey(&mut self, user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        self.observed.record(&format!("pubkey:{user}"));
        Ok(if user == "keyuser" {
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
        Ok(())
    }
}

/// 生成"CA 私钥 + 用户私钥 + 用户证书"三件套，返回两个私钥文件与证书文件路径。
fn write_cert_material(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let ca_key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
    let user_key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut builder =
        Builder::new(vec![0u8; 32], user_key.public_key(), now - 60, now + 3600).unwrap();
    builder.serial(1).unwrap();
    builder.key_id("transom-ci").unwrap();
    builder.cert_type(CertType::User).unwrap();
    builder.valid_principal("certuser").unwrap();
    builder.valid_principal("keyuser").unwrap();
    let certificate = builder.sign(&ca_key).unwrap();

    let key_path = dir.join("id_ed25519");
    let cert_path = dir.join("id_ed25519-cert.pub");
    user_key
        .write_openssh_file(&key_path, LineEnding::LF)
        .expect("写入测试私钥");
    std::fs::write(&cert_path, certificate.to_openssh().unwrap().as_bytes()).expect("写入测试证书");
    (key_path, cert_path)
}

async fn run_case(
    user: &str,
    cert_path: Option<std::path::PathBuf>,
    key_path: &std::path::Path,
) -> (
    Observed,
    Result<transom_net::SshSession, transom_core::AppError>,
) {
    let observed = Observed::new();
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
    let server_observed = observed.clone();
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let handle = server::run_stream(
            server_config,
            socket,
            CertServer {
                observed: server_observed,
            },
        )
        .await
        .unwrap();
        let _ = handle.await;
    });

    let connection = SshConnection {
        host: "127.0.0.1".into(),
        port,
        username: user.into(),
        auth: AuthMethod::PrivateKey {
            path: key_path.to_path_buf(),
            cert_path,
            save_passphrase: false,
        },
        options: SessionOptions {
            connect_timeout_secs: 5,
            keepalive_secs: 0,
            allow_keyboard_interactive: false,
            auto_reconnect: false,
            wol_mac: None,
        },
    };
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        connect_ssh(
            &connection,
            TerminalSize::new(80, 24),
            ConnectionSecrets::default(),
            Arc::new(PasswordPrompter::new("unused")),
            known_hosts,
        ),
    )
    .await
    .expect("连接超时");
    server_task.abort();
    (observed, result)
}

/// 配了证书就必须把证书发出去，且成功后不再多试一次裸公钥。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn certificate_is_preferred_and_not_followed_by_pubkey() {
    let dir = tempfile::tempdir().unwrap();
    let (key_path, cert_path) = write_cert_material(dir.path());
    let (observed, result) = run_case("certuser", Some(cert_path), &key_path).await;
    let session = result.expect("证书认证应当成功");
    drop(session.io.reader);
    drop(session.io.writer);
    let _ = session.io.control.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), session.join).await;

    assert_eq!(observed.events(), vec!["cert:certuser".to_string()]);
}

/// 服务器不认这张证书时回退裸公钥，而不是直接判定连接失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falls_back_to_pubkey_when_certificate_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (key_path, cert_path) = write_cert_material(dir.path());
    let (observed, result) = run_case("keyuser", Some(cert_path), &key_path).await;
    let session = result.expect("回退到公钥后认证应当成功");
    drop(session.io.reader);
    drop(session.io.writer);
    let _ = session.io.control.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), session.join).await;

    assert_eq!(
        observed.events(),
        vec!["cert:keyuser".to_string(), "pubkey:keyuser".to_string()]
    );
}

/// 没配证书时行为与改动前完全一致：只发公钥，不发证书。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_certificate_only_pubkey_is_offered() {
    let dir = tempfile::tempdir().unwrap();
    let (key_path, _cert_path) = write_cert_material(dir.path());
    let (observed, result) = run_case("keyuser", None, &key_path).await;
    let session = result.expect("公钥认证应当成功");
    drop(session.io.reader);
    drop(session.io.writer);
    let _ = session.io.control.close();
    let _ = tokio::time::timeout(Duration::from_secs(3), session.join).await;

    assert_eq!(observed.events(), vec!["pubkey:keyuser".to_string()]);
}

/// 证书路径写错要在认证阶段就给出可读错误，而不是静默退回公钥。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_certificate_file_reports_path() {
    let dir = tempfile::tempdir().unwrap();
    let (key_path, _cert_path) = write_cert_material(dir.path());
    let missing = dir.path().join("nope-cert.pub");
    let (_observed, result) = run_case("certuser", Some(missing.clone()), &key_path).await;
    let error = match result {
        Ok(_) => panic!("证书文件不存在时必须认证失败"),
        Err(error) => error,
    };
    let detail = format!("{error:?}");
    assert!(
        detail.contains("nope-cert.pub"),
        "错误信息里必须带证书路径，实际: {detail}"
    );
}
