use super::connection::*;
use super::entity::*;
use super::ssh::*;
use crate::ids::{FolderId, SessionId};
use std::path::PathBuf;

/// 自动重连是后加的：老会话文件的 options 缺该字段，必须按关闭降级。
#[test]
fn legacy_options_default_auto_reconnect_off() {
    let legacy =
        r#"{"connect_timeout_secs":15,"keepalive_secs":30,"allow_keyboard_interactive":true}"#;
    let options: SessionOptions = serde_json::from_str(legacy).unwrap();
    assert!(!options.auto_reconnect);
}

/// MAC 字段更晚加入：老会话文件的 options 没有它，必须按未配置降级而不是解析失败。
#[test]
fn legacy_options_load_without_wol_mac() {
    let legacy = r#"{
        "connect_timeout_secs":15,
        "keepalive_secs":30,
        "allow_keyboard_interactive":true,
        "auto_reconnect":true
    }"#;
    let options: SessionOptions = serde_json::from_str(legacy).unwrap();
    assert_eq!(options.wol_mac, None);

    let configured: SessionOptions = serde_json::from_str(
        r#"{"connect_timeout_secs":15,"keepalive_secs":0,"allow_keyboard_interactive":false,"wol_mac":"aa:bb:cc:dd:ee:ff"}"#,
    )
    .unwrap();
    assert_eq!(configured.wol_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
}

/// 证书字段是后加的：老会话文件里的私钥认证没有 `cert_path`，必须按"不带证书"降级。
#[test]
fn legacy_private_key_auth_loads_without_certificate() {
    let legacy = r#"{
        "kind": "private_key",
        "path": "/home/deploy/.ssh/id_ed25519",
        "save_passphrase": true
    }"#;
    let auth: AuthMethod = serde_json::from_str(legacy).unwrap();
    assert!(matches!(
        auth,
        AuthMethod::PrivateKey {
            cert_path: None,
            ..
        }
    ));
}

/// 跳板字段是后加的：老会话文件的 SSH 连接没有 `proxy_jump`，必须按直连降级。
#[test]
fn legacy_ssh_connection_loads_without_proxy_jump() {
    let legacy = r#"{
        "host": "10.0.0.9",
        "port": 22,
        "username": "deploy",
        "auth": {"kind": "agent"},
        "options": {"connect_timeout_secs": 15, "keepalive_secs": 30, "allow_keyboard_interactive": true}
    }"#;
    let ssh: SshConnection = serde_json::from_str(legacy).unwrap();
    assert_eq!(ssh.proxy_jump, None);
}

#[test]
fn proxy_jump_round_trips_through_json() {
    let jump_id = SessionId::new();
    let connection = SshConnection {
        host: "prod.internal".into(),
        port: 22,
        username: "deploy".into(),
        auth: AuthMethod::Agent,
        options: SessionOptions::default(),
        proxy_jump: Some(ProxyJumpConfig {
            jump_session_id: jump_id,
        }),
    };
    let json = serde_json::to_string(&connection).unwrap();
    let decoded: SshConnection = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded.proxy_jump.map(|jump| jump.jump_session_id),
        Some(jump_id)
    );
}

#[test]
fn session_round_trips_through_json() {
    let mut session = Session::new(
        "prod",
        Connection::Ssh(SshConnection {
            host: "example.com".into(),
            port: 2222,
            username: "deploy".into(),
            auth: AuthMethod::PrivateKey {
                path: PathBuf::from("/home/deploy/.ssh/id_ed25519"),
                cert_path: None,
                save_passphrase: true,
            },
            options: SessionOptions::default(),
            proxy_jump: None,
        }),
    );
    session.folder_id = Some(FolderId::new());

    let json = serde_json::to_string(&session).unwrap();
    let decoded: Session = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.id, session.id);
    assert_eq!(decoded.name, "prod");
    assert_eq!(decoded.folder_id, session.folder_id);
    if let Connection::Ssh(ssh) = decoded.connection {
        assert_eq!(ssh.port, 2222);
        assert!(matches!(ssh.auth, AuthMethod::PrivateKey { .. }));
    } else {
        panic!("Expected SSH connection");
    }
}

#[test]
fn local_connection_roundtrip() {
    let session = Session::new("local", Connection::Local(LocalConnection::default()));
    let json = serde_json::to_string(&session).unwrap();
    let decoded: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.name, "local");
    assert!(matches!(decoded.connection, Connection::Local(_)));
}

#[test]
fn startup_commands_ignore_blank_lines_and_round_trip() {
    let startup = StartupCommands::from_multiline("cd /srv\n\n  export A=\"x y\"  \n");
    assert_eq!(
        startup.commands,
        vec!["cd /srv".to_string(), "export A=\"x y\"".to_string()]
    );
    assert_eq!(startup.to_multiline(), "cd /srv\nexport A=\"x y\"");

    let mut session = Session::new("prod", Connection::Local(LocalConnection::default()));
    session.startup = startup;
    let json = serde_json::to_string(&session).unwrap();
    let decoded: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.startup.commands.len(), 2);
}

#[test]
fn serial_connection_round_trips_with_defaults() {
    let session = Session::new(
        "console",
        Connection::Serial(SerialConnection::new("/dev/ttyUSB0", 9600)),
    );
    let json = serde_json::to_string(&session).unwrap();
    let decoded: Session = serde_json::from_str(&json).unwrap();
    let Connection::Serial(serial) = decoded.connection else {
        panic!("应解析为串口连接");
    };
    assert_eq!(serial.port, "/dev/ttyUSB0");
    assert_eq!(serial.baud_rate, 9600);
    assert_eq!(serial.data_bits, SerialDataBits::Eight);
    assert_eq!(serial.parity, SerialParity::None);
    assert_eq!(serial.stop_bits, SerialStopBits::One);
    assert_eq!(serial.flow_control, SerialFlowControl::None);
}

#[test]
fn serial_defaults_fill_missing_optional_fields() {
    let value = serde_json::json!({
        "kind": "serial",
        "port": "/dev/ttyS0",
        "baud_rate": 115200
    });
    let serial: SerialConnection = serde_json::from_value(value).unwrap();
    assert_eq!(serial.data_bits.bits(), 8);
}

#[test]
fn telnet_connection_round_trips_with_defaults() {
    let session = Session::new(
        "bbs",
        Connection::Telnet(TelnetConnection::new("bbs.example.com")),
    );
    let json = serde_json::to_string(&session).unwrap();
    let decoded: Session = serde_json::from_str(&json).unwrap();
    let Connection::Telnet(telnet) = decoded.connection else {
        panic!("应解析为 Telnet 连接");
    };
    assert_eq!(telnet.host, "bbs.example.com");
    assert_eq!(telnet.port, 23);
    assert_eq!(telnet.encoding, TextEncoding::Utf8);
    assert_eq!(telnet.options.connect_timeout_secs, 15);
    assert_eq!(telnet.options.keepalive_secs, 0);
    assert!(!telnet.options.auto_reconnect);
}

/// 老会话文件没有 encoding/options 字段，必须按默认值降级而不是解析失败。
#[test]
fn telnet_defaults_fill_missing_optional_fields() {
    let value = serde_json::json!({
        "kind": "telnet",
        "host": "10.0.0.1",
        "port": 2300,
        "encoding": "gbk"
    });
    let telnet: TelnetConnection = serde_json::from_value(value).unwrap();
    assert_eq!(telnet.host, "10.0.0.1");
    assert_eq!(telnet.port, 2300);
    assert_eq!(telnet.encoding, TextEncoding::Gbk);
    assert_eq!(telnet.options, TelnetOptions::default());
}

#[test]
fn missing_startup_field_defaults_to_empty() {
    let session = Session::new("local", Connection::Local(LocalConnection::default()));
    let mut value = serde_json::to_value(session).unwrap();
    value.as_object_mut().unwrap().remove("startup");
    let decoded: Session = serde_json::from_value(value).unwrap();
    assert!(decoded.startup.is_empty());
}
