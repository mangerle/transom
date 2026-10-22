//! OpenSSH 配置文件（~/.ssh/config）解析模块。

use crate::session::{AuthMethod, Connection, Session, SessionOptions, SshConnection};
use std::path::{Path, PathBuf};

/// 解析 SSH 配置文件内容，使用当前用户家目录展开路径
pub fn parse_ssh_config(content: &str) -> Vec<Session> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    parse_ssh_config_with_home(content, home.as_deref())
}

/// 解析 SSH 配置文件内容，支持自定义家目录
pub fn parse_ssh_config_with_home(content: &str, home: Option<&Path>) -> Vec<Session> {
    let mut sessions = Vec::new();
    let mut current_hosts: Vec<String> = Vec::new();
    let mut current_hostname: Option<String> = None;
    let mut current_user: Option<String> = None;
    let mut current_port: Option<u16> = None;
    let mut current_identity: Option<PathBuf> = None;

    let flush_host = |sessions: &mut Vec<Session>,
                      hosts: &mut Vec<String>,
                      hostname: &mut Option<String>,
                      user: &mut Option<String>,
                      port: &mut Option<u16>,
                      identity: &mut Option<PathBuf>| {
        if hosts.is_empty() {
            return;
        }
        for host in hosts.drain(..) {
            // 跳过通配符主机规则，如 *, *.*, ?
            if host.contains('*') || host.contains('?') {
                continue;
            }
            let target_host = hostname.clone().unwrap_or_else(|| host.clone());
            let target_port = port.unwrap_or(22);
            let target_user = user.clone().unwrap_or_else(|| {
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "root".to_string())
            });

            let auth = if let Some(key_path) = identity.clone() {
                AuthMethod::PrivateKey {
                    path: key_path,
                    save_passphrase: false,
                }
            } else {
                AuthMethod::Agent
            };

            let connection = Connection::Ssh(SshConnection {
                host: target_host,
                port: target_port,
                username: target_user,
                auth,
                options: SessionOptions::default(),
            });

            let mut session = Session::new(host, connection);
            session.group = Some("SSH Config".to_string());
            sessions.push(session);
        }
        *hostname = None;
        *user = None;
        *port = None;
        *identity = None;
    };

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = if let Some((k, v)) = line.split_once('=') {
            (k.trim(), v.trim())
        } else if let Some((k, v)) = line.split_once(char::is_whitespace) {
            (k.trim(), v.trim())
        } else {
            continue;
        };

        if key.eq_ignore_ascii_case("Host") {
            flush_host(
                &mut sessions,
                &mut current_hosts,
                &mut current_hostname,
                &mut current_user,
                &mut current_port,
                &mut current_identity,
            );
            for h in value.split_whitespace() {
                current_hosts.push(h.to_string());
            }
        } else if key.eq_ignore_ascii_case("HostName") {
            current_hostname = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("User") {
            current_user = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("Port") {
            if let Ok(p) = value.parse::<u16>() {
                current_port = Some(p);
            }
        } else if key.eq_ignore_ascii_case("IdentityFile") {
            let path_str = value.trim_matches('"');
            let path = if let Some(stripped) = path_str.strip_prefix("~/") {
                if let Some(home) = home {
                    home.join(stripped)
                } else {
                    PathBuf::from(path_str)
                }
            } else {
                PathBuf::from(path_str)
            };
            current_identity = Some(path);
        }
    }

    flush_host(
        &mut sessions,
        &mut current_hosts,
        &mut current_hostname,
        &mut current_user,
        &mut current_port,
        &mut current_identity,
    );

    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_config() {
        let sample = r#"
# 全局或者注释
Host dev-box
    HostName 192.168.1.50
    User developer
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host prod1 prod2
    HostName example.com
    User admin

Host *
    User defaultuser
"#;
        let home = Path::new("/custom/home");
        let sessions = parse_ssh_config_with_home(sample, Some(home));
        assert_eq!(sessions.len(), 3);

        let dev = &sessions[0];
        assert_eq!(dev.name, "dev-box");
        assert_eq!(dev.group.as_deref(), Some("SSH Config"));
        if let Connection::Ssh(ssh) = &dev.connection {
            assert_eq!(ssh.host, "192.168.1.50");
            assert_eq!(ssh.username, "developer");
            assert_eq!(ssh.port, 2222);
            assert_eq!(
                ssh.auth,
                AuthMethod::PrivateKey {
                    path: PathBuf::from("/custom/home/.ssh/id_ed25519"),
                    save_passphrase: false,
                }
            );
        } else {
            panic!("Expected SSH connection");
        }

        let p1 = &sessions[1];
        assert_eq!(p1.name, "prod1");
        if let Connection::Ssh(ssh) = &p1.connection {
            assert_eq!(ssh.host, "example.com");
            assert_eq!(ssh.username, "admin");
            assert_eq!(ssh.port, 22);
            assert_eq!(ssh.auth, AuthMethod::Agent);
        }

        let p2 = &sessions[2];
        assert_eq!(p2.name, "prod2");
    }
}
