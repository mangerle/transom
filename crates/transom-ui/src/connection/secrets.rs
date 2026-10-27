//! 连接凭据收集与跳板机链路凭据汇总。

use crate::palette::localized_error;
use transom_core::{
    AuthMethod, Connection, ConnectionSecrets, SecretKey, SecretKind, SecretStore, Session,
};

/// 连接目标的日志用描述：`user@host:port`，本地与串口按各自口径。
pub(crate) fn describe_target(session: &Session) -> String {
    match &session.connection {
        Connection::Ssh(ssh) => format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
        Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
        Connection::Local(local) => {
            format!("本地终端 {}", local.shell.as_deref().unwrap_or("sh"))
        }
        Connection::Serial(serial) => format!("串口 {}@{}", serial.port, serial.baud_rate),
        _ => "未知连接类型".to_string(),
    }
}

/// 按认证方式收集钥匙串中已保存的连接凭据。
pub(crate) fn collect_connection_secrets(
    secret_store: &dyn SecretStore,
    session: &Session,
) -> (ConnectionSecrets, Option<String>) {
    let mut secrets = ConnectionSecrets::default();
    let mut error = None;
    let Connection::Ssh(ssh) = &session.connection else {
        return (secrets, error);
    };
    match &ssh.auth {
        AuthMethod::Password { .. } => {
            let key = SecretKey::new(session.secret_owner(), SecretKind::Password);
            match secret_store.get(&key) {
                Ok(Some(secret)) => secrets.password = Some(secret),
                Ok(None) => {}
                Err(err) => error = Some(localized_error(&err)),
            }
        }
        AuthMethod::PrivateKey {
            save_passphrase, ..
        } => {
            if *save_passphrase {
                let key = SecretKey::new(session.secret_owner(), SecretKind::KeyPassphrase);
                match secret_store.get(&key) {
                    Ok(Some(secret)) => secrets.key_passphrase = Some(secret),
                    Ok(None) => {}
                    Err(err) => error = Some(localized_error(&err)),
                }
            }
        }
        AuthMethod::Agent => {}
        _ => {}
    }
    (secrets, error)
}

/// 连接派发时随会话一起下发的敏感与链路载荷。
pub(crate) struct ConnectPayload {
    pub(crate) jumps: Vec<transom_core::SshHop>,
    pub(crate) proxy: transom_core::OutboundProxy,
    pub(crate) secrets: ConnectionSecrets,
}

/// 按跳板链顺序收集每一跳的凭据。
pub(crate) fn collect_jump_hops(
    secret_store: &dyn SecretStore,
    chain: &[Session],
) -> (Vec<transom_core::SshHop>, Option<String>) {
    let mut hops = Vec::with_capacity(chain.len());
    let mut error = None;
    for session in chain {
        let Connection::Ssh(ssh) = &session.connection else {
            continue;
        };
        let (secrets, hop_error) = collect_connection_secrets(secret_store, session);
        if error.is_none() {
            error = hop_error;
        }
        hops.push(transom_core::SshHop {
            connection: ssh.clone(),
            secrets,
        });
    }
    (hops, error)
}

/// 打开认证方式上的"保存口令"标记。
pub(crate) fn enable_auth_save(auth: &mut AuthMethod) -> bool {
    match auth {
        AuthMethod::Password { save } => {
            *save = true;
            true
        }
        AuthMethod::PrivateKey {
            save_passphrase, ..
        } => {
            *save_passphrase = true;
            true
        }
        _ => false,
    }
}
