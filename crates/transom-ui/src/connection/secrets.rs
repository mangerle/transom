//! 连接凭据收集与跳板机链路凭据汇总。

use crate::palette::localized_error;
use transom_core::{
    AuthMethod, Connection, ConnectionSecrets, SecretKey, SecretKind, SecretStore, Session,
};

/// 连接目标的日志用描述：`user@host:port`，本地与串口按各自口径。
pub(crate) fn describe_target(session: &Session) -> String {
    match &session.connection {
        Connection::Ssh(ssh) => format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
        Connection::Mosh(mosh) => format!("mosh {}@{}:{}", mosh.username, mosh.host, mosh.ssh_port),
        Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
        Connection::Local(local) => {
            format!("本地终端 {}", local.shell.as_deref().unwrap_or("sh"))
        }
        Connection::Serial(serial) => format!("串口 {}@{}", serial.port, serial.baud_rate),
        _ => "未知连接类型".to_string(),
    }
}

/// 会话的对端主机地址；本地终端与串口没有主机可言，返回 `None` 供界面隐藏入口。
///
/// 设计原理（Why）：
/// - 实现初衷：右键"复制主机地址"的用途是把地址原样填进别处（命令、白名单、表格），
///   所以只给 `host` 字段本身，不像 `describe_target` 那样拼用户名与端口——多出来的
///   成分得手工删掉，比少给更难用；
/// - 核心优势：三种带主机的连接（SSH、Mosh、Telnet）单点收口，端口字段各自不同
///   （`port` / `ssh_port` / `port`）的差异被这里吸收，调用方只做"有没有"的判断；
/// - 代价与局限：跳板链（`proxy_jump`）刻意不并入，那是路径而不是本会话的地址；
///   空 `host` 也归入 `None`，避免复制出一个空字符串却提示"已复制"。
pub(crate) fn session_host(session: &Session) -> Option<&str> {
    let host = match &session.connection {
        Connection::Ssh(ssh) => ssh.host.as_str(),
        Connection::Mosh(mosh) => mosh.host.as_str(),
        Connection::Telnet(telnet) => telnet.host.as_str(),
        _ => return None,
    };
    (!host.is_empty()).then_some(host)
}

/// 按认证方式收集钥匙串中已保存的连接凭据。
pub(crate) fn collect_connection_secrets(
    secret_store: &dyn SecretStore,
    session: &Session,
) -> (ConnectionSecrets, Option<String>) {
    let mut secrets = ConnectionSecrets::default();
    let mut error = None;
    let auth = match &session.connection {
        Connection::Ssh(ssh) => &ssh.auth,
        Connection::Mosh(mosh) => &mosh.auth,
        _ => return (secrets, error),
    };
    match auth {
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

#[cfg(test)]
mod tests {
    use super::session_host;
    use transom_core::{
        Connection, LocalConnection, MoshConnection, SerialConnection, Session, SshConnection,
        TelnetConnection,
    };

    fn session(connection: Connection) -> Session {
        Session::new("测试会话", connection)
    }

    #[test]
    fn host_bearing_connections_expose_their_address() {
        let ssh = session(Connection::Ssh(SshConnection::new("10.0.0.5", "root")));
        assert_eq!(
            session_host(&ssh),
            Some("10.0.0.5"),
            "只给 host，不带用户名与端口"
        );
        let mosh = session(Connection::Mosh(MoshConnection::new("mosh.example", "u")));
        assert_eq!(session_host(&mosh), Some("mosh.example"));
        let telnet = session(Connection::Telnet(TelnetConnection::new("bbs.example")));
        assert_eq!(session_host(&telnet), Some("bbs.example"));
    }

    #[test]
    fn local_and_serial_have_no_address() {
        let local = session(Connection::Local(LocalConnection::default()));
        assert_eq!(session_host(&local), None);
        let serial = session(Connection::Serial(SerialConnection::new(
            "/dev/ttyS0",
            9_600,
        )));
        assert_eq!(session_host(&serial), None);
    }

    #[test]
    fn empty_host_is_treated_as_absent() {
        let broken = session(Connection::Ssh(SshConnection::new("", "root")));
        assert_eq!(
            session_host(&broken),
            None,
            "空地址不该给出一个可复制的空串"
        );
    }
}
