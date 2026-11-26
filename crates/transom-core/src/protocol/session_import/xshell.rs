//! Xshell 会话文件（`.xsh`）解析器与目录扫描器。
//!
//! Xshell 每个会话对应一个 `.xsh`（INI 格式）文件，分组通过操作系统目录树体现。
//! 常见编码包括 UTF-8（带或不带 BOM）与 UTF-16LE（带 BOM）。

use super::{
    ImportedSessionEntry, MAX_IMPORT_FILE_BYTES, decode_text_bytes, normalize_folder_path,
};
use crate::model::session::{
    AuthMethod, Connection, FtpConnection, RdpConnection, Session, SshConnection, TelnetConnection,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 解析单个 Xshell `.xsh` 文件内容，以 `name` 作为会话名。
pub fn parse_xshell_session(name: &str, content: &str) -> Option<Session> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return None;
    }

    let sections = parse_ini_sections(content);
    let conn_sec = sections.get("connection")?;
    let host = conn_sec
        .get("host")
        .map(String::as_str)
        .unwrap_or("")
        .trim();
    if host.is_empty() {
        return None;
    }

    let protocol = conn_sec
        .get("protocol")
        .map(|s| s.trim().to_ascii_uppercase())
        .unwrap_or_else(|| "SSH".to_string());
    let port = conn_sec
        .get("port")
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p > 0);

    let auth_sec = sections.get("connection:authentication");
    let username = auth_sec
        .and_then(|s| s.get("username"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let connection = match protocol.as_str() {
        "SSH" | "" => {
            let method = auth_sec
                .and_then(|s| s.get("method"))
                .map(|s| s.trim().to_ascii_lowercase())
                .unwrap_or_default();
            let user_key = auth_sec
                .and_then(|s| s.get("userkey"))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            let auth = if method.contains("public") || user_key.is_some() {
                user_key
                    .map(|path| AuthMethod::PrivateKey {
                        path: PathBuf::from(path),
                        save_passphrase: false,
                        cert_path: None,
                    })
                    .unwrap_or(AuthMethod::Agent)
            } else {
                AuthMethod::Password { save: false }
            };

            let mut ssh = SshConnection::new(host, username);
            ssh.port = port.unwrap_or(22);
            ssh.auth = auth;
            Connection::Ssh(ssh)
        }
        "TELNET" => {
            let mut telnet = TelnetConnection::new(host);
            telnet.port = port.unwrap_or(23);
            Connection::Telnet(telnet)
        }
        "FTP" | "SFTP" => {
            let mut ftp = FtpConnection::new(host, username);
            ftp.port = port.unwrap_or(21);
            Connection::Ftp(ftp)
        }
        "RDP" => {
            let mut rdp = RdpConnection::new(host, username);
            rdp.port = port.unwrap_or(3389);
            Connection::Rdp(rdp)
        }
        _ => return None,
    };

    Some(Session::new(trimmed_name, connection))
}

/// 从磁盘读取并解析单个 `.xsh` 文件；`base_dir` 用于计算相对分组路径。
pub fn parse_xshell_file(path: &Path, base_dir: Option<&Path>) -> Option<ImportedSessionEntry> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMPORT_FILE_BYTES {
        log::warn!(
            "跳过无效或过大的 Xshell 会话文件: {} ({} bytes)",
            path.display(),
            meta.len()
        );
        return None;
    }

    let bytes = std::fs::read(path).ok()?;
    let content = decode_text_bytes(&bytes);
    let session_name = path.file_stem()?.to_string_lossy().to_string();
    let session = parse_xshell_session(&session_name, &content)?;

    let folder_path = base_dir
        .and_then(|root| path.parent().and_then(|p| p.strip_prefix(root).ok()))
        .and_then(|rel| normalize_folder_path(&rel.to_string_lossy()));

    Some(ImportedSessionEntry {
        session,
        folder_path,
    })
}

/// 迭代扫描指定目录下的所有 `.xsh` 文件，保留子目录层级结构。
///
/// 使用显式堆栈代替递归，避免深层符号链接或目录树导致栈溢出。
pub fn scan_xshell_directory(root: &Path) -> Vec<ImportedSessionEntry> {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            log::warn!("无法读取 Xshell 会话目录: {}", dir.display());
            continue;
        };
        let mut children: Vec<PathBuf> =
            read_dir.filter_map(|e| e.ok().map(|d| d.path())).collect();
        children.sort();
        for path in children {
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xsh"))
                && let Some(entry) = parse_xshell_file(&path, Some(root))
            {
                entries.push(entry);
            }
        }
    }

    entries
}

/// 轻量级 INI 解析：节名与键名统一转为小写以兼容不同版本 Xshell 大小写差异。
fn parse_ini_sections(content: &str) -> HashMap<String, HashMap<String, String>> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = line[1..line.len().saturating_sub(1)]
                .trim()
                .to_ascii_lowercase();
            sections.entry(current.clone()).or_default();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            sections
                .entry(current.clone())
                .or_default()
                .insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xshell_ssh_session() {
        let sample = "\
[SessionInfo]
Version=7.0
[CONNECTION]
Protocol=SSH
Host=10.20.30.40
Port=22022
[CONNECTION:AUTHENTICATION]
Method=Password
UserName=admin
";
        let session = parse_xshell_session("prod-api", sample).expect("应成功解析 Xshell SSH 会话");
        assert_eq!(session.name, "prod-api");
        match session.connection {
            Connection::Ssh(ssh) => {
                assert_eq!(ssh.host, "10.20.30.40");
                assert_eq!(ssh.port, 22022);
                assert_eq!(ssh.username, "admin");
                assert!(matches!(ssh.auth, AuthMethod::Password { .. }));
            }
            other => panic!("预期为 SSH，实际: {other:?}"),
        }
    }

    #[test]
    fn decodes_utf16le_bom_xshell_file() {
        let text = "[CONNECTION]\r\nProtocol=SSH\r\nHost=172.16.0.5\r\nPort=22\r\n[CONNECTION:AUTHENTICATION]\r\nMethod=Public Key\r\nUserName=ubuntu\r\nUserKey=/home/ubuntu/.ssh/id_rsa\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let decoded = decode_text_bytes(&bytes);
        let session = parse_xshell_session("utf16-node", &decoded).expect("应能解析 UTF-16LE 内容");
        match session.connection {
            Connection::Ssh(ssh) => {
                assert_eq!(ssh.host, "172.16.0.5");
                assert_eq!(ssh.username, "ubuntu");
                assert!(matches!(ssh.auth, AuthMethod::PrivateKey { .. }));
            }
            other => panic!("预期为 SSH，实际: {other:?}"),
        }
    }
}
