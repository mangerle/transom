//! parse 相关的引擎候选发现。

use std::path::PathBuf;
use transom_core::{ContainerEndpoint, TlsConfig};

/// 解析常见的端点字符串。
pub fn parse_endpoint(value: &str) -> Option<ContainerEndpoint> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(path) = value.strip_prefix("unix://") {
        return Some(ContainerEndpoint::Unix {
            path: PathBuf::from(path),
        });
    }
    if let Some(path) = value.strip_prefix("npipe://") {
        return Some(ContainerEndpoint::NamedPipe {
            path: path.to_string(),
        });
    }
    if value.starts_with("tcp://") || value.starts_with("http://") || value.starts_with("https://")
    {
        return Some(ContainerEndpoint::Tcp {
            url: value.to_string(),
            // https 明确要求 TLS；证书路径由环境变量或默认目录补充。
            tls: value.starts_with("https://").then(TlsConfig::default),
        });
    }
    if let Some(rest) = value.strip_prefix("ssh://") {
        return Some(parse_ssh_endpoint(rest));
    }
    if value.starts_with('/') || value.starts_with("~/") {
        return Some(ContainerEndpoint::Unix {
            path: PathBuf::from(value),
        });
    }
    None
}

/// 解析 SSH 地址字符串（不含 `ssh://` 前缀）。
///
/// 支持格式：`host`、`host:port`、`user@host`、`user@host:port`。
/// 端口必须是合法的 u16，否则整个字符串按 host 处理。
fn parse_ssh_endpoint(s: &str) -> ContainerEndpoint {
    // 先分离 user 部分
    let (user, rest) = if let Some(idx) = s.find('@') {
        let u = &s[..idx];
        (
            if u.is_empty() {
                None
            } else {
                Some(u.to_string())
            },
            &s[idx + 1..],
        )
    } else {
        (None, s)
    };
    // 再分离 host 和 port
    let (host, port) = if let Some((h, p)) = rest.rsplit_once(':') {
        if let Ok(port_num) = p.parse::<u16>() {
            (h.to_string(), Some(port_num))
        } else {
            (rest.to_string(), None)
        }
    } else {
        (rest.to_string(), None)
    };
    ContainerEndpoint::Ssh {
        host,
        port,
        user,
        identity_file: None,
        password: None,
        use_sudo: false,
        sudo_password: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_common_endpoint_forms() {
        assert_eq!(
            parse_endpoint("unix:///var/run/docker.sock"),
            Some(ContainerEndpoint::Unix {
                path: PathBuf::from("/var/run/docker.sock")
            })
        );
        assert_eq!(
            parse_endpoint("npipe:////./pipe/docker_engine"),
            Some(ContainerEndpoint::NamedPipe {
                path: "//./pipe/docker_engine".to_string()
            })
        );
        assert_eq!(
            parse_endpoint("tcp://127.0.0.1:2375"),
            Some(ContainerEndpoint::Tcp {
                url: "tcp://127.0.0.1:2375".to_string(),
                tls: None,
            })
        );
        assert!(matches!(
            parse_endpoint("ssh://user@host"),
            Some(ContainerEndpoint::Ssh { .. })
        ));
        assert!(matches!(
            parse_endpoint("https://docker.example.com:2376"),
            Some(ContainerEndpoint::Tcp { tls: Some(_), .. })
        ));
        assert!(matches!(
            parse_endpoint("tcp://127.0.0.1:2375"),
            Some(ContainerEndpoint::Tcp { tls: None, .. })
        ));
        assert_eq!(parse_endpoint("   "), None);
        assert_eq!(parse_endpoint("relative/path"), None);
    }

    #[test]
    fn parses_ssh_endpoint_formats() {
        // host only
        let ep = parse_ssh_endpoint("myhost.example.com");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, port: None, user: None, .. } if host == "myhost.example.com")
        );

        // user@host
        let ep = parse_ssh_endpoint("admin@192.168.1.100");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, user: Some(u), port: None, .. } if host == "192.168.1.100" && u == "admin")
        );

        // host:port
        let ep = parse_ssh_endpoint("myhost:2222");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, port: Some(2222), user: None, .. } if host == "myhost")
        );

        // user@host:port
        let ep = parse_ssh_endpoint("root@10.0.0.1:22");
        assert!(
            matches!(&ep, ContainerEndpoint::Ssh { host, port: Some(22), user: Some(u), .. } if host == "10.0.0.1" && u == "root")
        );
    }
}
