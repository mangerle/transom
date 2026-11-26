//! MobaXterm 会话文件（`.mxtsessions`）解析器。
//!
//! MobaXterm 导出的 `.mxtsessions` 为 INI 结构：
//! - `[Bookmarks]` / `[Bookmarks_1]` / `[Bookmarks_2]` ... 表示各个分组节；
//! - `SubRep=<目录>` 使用 `\` 分隔分组层级；
//! - 会话行格式为 `<名称>=#<图标>#<协议号>%<主机>%<端口>%<用户名>%...`。

use super::{ImportedSessionEntry, normalize_folder_path};
use crate::model::session::{
    AuthMethod, Connection, FtpConnection, RdpConnection, Session, SshConnection, TelnetConnection,
};
use std::path::PathBuf;

/// MobaXterm 协议编号常量。
const PROTO_SSH: &str = "0";
const PROTO_TELNET: &str = "1";
const PROTO_RDP: &str = "3";
const PROTO_FTP: &str = "5";

/// 解析 MobaXterm `.mxtsessions` 文本内容，返回识别到的会话清单。
///
/// 不支持的协议类型（如 VNC、Browser 等）或缺少主机地址的无效条目会被静默跳过。
pub fn parse_mobaxterm_sessions(content: &str) -> Vec<ImportedSessionEntry> {
    let mut entries = Vec::new();
    let mut current_section = String::new();
    let mut current_folder: Option<String> = None;
    let mut pending_lines: Vec<(&str, &str)> = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            flush_section(
                &current_section,
                current_folder.as_deref(),
                &pending_lines,
                &mut entries,
            );
            pending_lines.clear();
            current_section = line[1..line.len().saturating_sub(1)].trim().to_string();
            current_folder = None;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.eq_ignore_ascii_case("SubRep") {
            current_folder = normalize_folder_path(value);
        } else if !key.eq_ignore_ascii_case("ImgNum") {
            pending_lines.push((key, value));
        }
    }

    flush_section(
        &current_section,
        current_folder.as_deref(),
        &pending_lines,
        &mut entries,
    );
    entries
}

/// 处理单个 `[Bookmarks*]` 节内收集到的会话条目。
fn flush_section(
    section: &str,
    folder_path: Option<&str>,
    lines: &[(&str, &str)],
    out: &mut Vec<ImportedSessionEntry>,
) {
    if !section.to_ascii_lowercase().starts_with("bookmarks") {
        return;
    }
    let folder = folder_path.map(str::to_string);
    for (name, value) in lines {
        if let Some(session) = parse_bookmark_line(name, value) {
            out.push(ImportedSessionEntry {
                session,
                folder_path: folder.clone(),
            });
        }
    }
}

/// 解析单条 MobaXterm 书签行：`#<图标>#<协议号>%<字段1>%<字段2>...`
fn parse_bookmark_line(name: &str, value: &str) -> Option<Session> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() || !value.starts_with('#') {
        return None;
    }
    // 跳过 `#<icon>#` 前缀
    let after_first_hash = &value[1..];
    let (_, payload) = after_first_hash.split_once('#')?;
    let fields: Vec<&str> = payload.split('%').collect();
    let proto = fields.first().copied()?.trim();
    let host = fields.get(1).copied().unwrap_or("").trim();
    if host.is_empty() {
        return None;
    }

    let connection = match proto {
        PROTO_SSH => build_ssh_connection(host, &fields),
        PROTO_TELNET => build_telnet_connection(host, &fields),
        PROTO_FTP => build_ftp_connection(host, &fields),
        PROTO_RDP => build_rdp_connection(host, &fields),
        _ => return None,
    };

    Some(Session::new(trimmed_name, connection))
}

fn build_ssh_connection(host: &str, fields: &[&str]) -> Connection {
    let port = fields
        .get(2)
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(22);
    let username = fields.get(3).copied().unwrap_or("").trim().to_string();
    // MobaXterm SSH 条目第 14 个字段（索引 14）常存放私钥路径；若为真实路径则按私钥认证，否则默认密码认证。
    let key_path = fields
        .get(14)
        .copied()
        .map(str::trim)
        .filter(|path| !path.is_empty() && *path != "0" && *path != "_Profile_");
    let auth = match key_path {
        Some(path) => AuthMethod::PrivateKey {
            path: PathBuf::from(path),
            save_passphrase: false,
            cert_path: None,
        },
        None => AuthMethod::Password { save: false },
    };

    let mut conn = SshConnection::new(host, username);
    conn.port = port;
    conn.auth = auth;
    Connection::Ssh(conn)
}

fn build_telnet_connection(host: &str, fields: &[&str]) -> Connection {
    let port = fields
        .get(2)
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(23);
    let mut conn = TelnetConnection::new(host);
    conn.port = port;
    Connection::Telnet(conn)
}

fn build_ftp_connection(host: &str, fields: &[&str]) -> Connection {
    let port = fields
        .get(2)
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(21);
    let username = fields.get(3).copied().unwrap_or("").trim().to_string();
    let mut conn = FtpConnection::new(host, username);
    conn.port = port;
    Connection::Ftp(conn)
}

fn build_rdp_connection(host: &str, fields: &[&str]) -> Connection {
    let port = fields
        .get(2)
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(3389);
    let username = fields.get(3).copied().unwrap_or("").trim().to_string();
    let mut conn = RdpConnection::new(host, username);
    conn.port = port;
    Connection::Rdp(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mobaxterm_ssh_and_folders() {
        let sample = r#"
[Bookmarks]
SubRep=
ImgNum=42
Bastion=#109#0%10.0.0.1%22%ops%0%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1%-1#0# #-1

[Bookmarks_1]
SubRep=Prod\Web
ImgNum=41
Web-01=#109#0%192.168.10.11%2222%deploy%0%-1%-1%%%%%0%0%0%/home/ops/.ssh/id_ed25519%%-1%0%0%0%%1080%%0%0%1#MobaFont
Win-Jump=#91#3%192.168.10.20%3389%Administrator%0%0%0#MobaFont
Ignored-Vnc=#129#4%192.168.10.30%5900%%0#MobaFont
"#;
        let result = parse_mobaxterm_sessions(sample);
        assert_eq!(result.len(), 3);

        assert_eq!(result[0].session.name, "Bastion");
        assert_eq!(result[0].folder_path, None);
        match &result[0].session.connection {
            Connection::Ssh(ssh) => {
                assert_eq!(ssh.host, "10.0.0.1");
                assert_eq!(ssh.port, 22);
                assert_eq!(ssh.username, "ops");
                assert!(matches!(ssh.auth, AuthMethod::Password { .. }));
            }
            other => panic!("预期为 SSH，实际: {other:?}"),
        }

        assert_eq!(result[1].session.name, "Web-01");
        assert_eq!(result[1].folder_path.as_deref(), Some("Prod/Web"));
        match &result[1].session.connection {
            Connection::Ssh(ssh) => {
                assert_eq!(ssh.host, "192.168.10.11");
                assert_eq!(ssh.port, 2222);
                assert_eq!(ssh.username, "deploy");
                assert!(matches!(
                    &ssh.auth,
                    AuthMethod::PrivateKey { path, .. } if path == &PathBuf::from("/home/ops/.ssh/id_ed25519")
                ));
            }
            other => panic!("预期为 SSH，实际: {other:?}"),
        }

        assert_eq!(result[2].session.name, "Win-Jump");
        assert_eq!(result[2].folder_path.as_deref(), Some("Prod/Web"));
        assert!(matches!(result[2].session.connection, Connection::Rdp(_)));
    }
}
