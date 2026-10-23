use crate::forward::ForwardRule;
use crate::ids::{FolderId, SessionId};
use crate::terminal::TerminalProfile;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    /// 所属文件夹；`None` 表示位于会话列表根层级。
    #[serde(default)]
    pub folder_id: Option<FolderId>,
    pub connection: Connection,
    pub terminal: TerminalProfile,
    #[serde(default)]
    pub forwards: Vec<ForwardRule>,
}

impl Session {
    pub fn new(name: impl Into<String>, connection: Connection) -> Self {
        Self {
            id: SessionId::new(),
            name: name.into(),
            folder_id: None,
            connection,
            terminal: TerminalProfile::default(),
            forwards: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LocalConnection {
    pub shell: Option<String>,
    pub working_dir: Option<PathBuf>,
}

impl LocalConnection {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Connection {
    Ssh(SshConnection),
    Local(LocalConnection),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConnection {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
    #[serde(default)]
    pub options: SessionOptions,
}

impl SshConnection {
    pub fn new(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 22,
            username: username.into(),
            auth: AuthMethod::Agent,
            options: SessionOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthMethod {
    Password {
        #[serde(default)]
        save: bool,
    },
    PrivateKey {
        path: PathBuf,
        #[serde(default)]
        save_passphrase: bool,
    },
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOptions {
    pub connect_timeout_secs: u64,
    /// `0` disables keepalive messages.
    pub keepalive_secs: u64,
    pub allow_keyboard_interactive: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            keepalive_secs: 30,
            allow_keyboard_interactive: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    save_passphrase: true,
                },
                options: SessionOptions::default(),
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
}
