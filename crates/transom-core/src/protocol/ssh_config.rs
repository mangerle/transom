//! OpenSSH 配置文件（~/.ssh/config）解析模块。
//!
//! # Include 语义
//! OpenSSH 把 `Include` 当作文本插入：包含进来的内容出现在 Include 所在的
//! 位置，因此一个 `Host` 块可以跨文件继续（主文件写 `Host a`，被包含文件
//! 追加 `Port 2200`，两行属于同一个块）。本模块按同样方式递归展开，而不是
//! "每个文件独立解析后再拼接"。
//!
//! 路径口径与 OpenSSH 一致：相对路径按 `~/.ssh` 解析，而不是按被包含文件
//! 所在目录；通配符只展开最后一级路径分量（`config.d/*` 可用，`*/*` 这类
//! 多级通配会被忽略并留痕），因为实际配置里 Include 的形态几乎都是目录
//! 通配。代价是极少数多级通配配置会被静默跳过——日志里能查到。

use crate::model::session::{AuthMethod, Connection, Session, SessionOptions, SshConnection};
use globset::Glob;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Include 递归深度上限：与 OpenSSH 的 16 层保持一致。
const MAX_INCLUDE_DEPTH: usize = 16;

/// 解析 SSH 配置文件内容，使用当前用户家目录展开路径与相对 Include。
pub fn parse_ssh_config(content: &str) -> Vec<Session> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    parse_ssh_config_with_home(content, home.as_deref())
}

/// 解析 SSH 配置文件内容，支持自定义家目录。
///
/// `home` 同时用于 `~/` 路径展开与相对 Include 的基准目录（`home/.ssh`）；
/// 为 `None` 时 `~` 与相对 Include 都无法解析，会按忽略处理并留痕。
pub fn parse_ssh_config_with_home(content: &str, home: Option<&Path>) -> Vec<Session> {
    let mut parser = ConfigParser {
        sessions: Vec::new(),
        hosts: Vec::new(),
        hostname: None,
        user: None,
        port: None,
        identity: None,
        home,
        include_stack: HashSet::new(),
    };
    parser.parse_content(content, 0);
    // 只有最外层解析结束才收尾：被包含文件的尾部 Host 块按文本插入语义
    // 延续到父文件，提前 flush 会把同一个块拆成两个会话。
    parser.flush_host();
    parser.sessions
}

/// 解析过程中的累积状态。
///
/// 设计原理（Why）：原实现用六个平铺局部变量加一个闭包传递状态，加入
/// Include 递归后参数列表会涨到七个；收进类型后，递归只需 `&mut self`，
/// 状态字段与 flush 逻辑也能保持单一出处。
struct ConfigParser<'a> {
    sessions: Vec<Session>,
    hosts: Vec<String>,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity: Option<PathBuf>,
    home: Option<&'a Path>,
    /// 当前递归栈上的规范路径：只用于阻断循环引用，出栈即移除——
    /// 同一文件在配置里被包含两次是合法写法（OpenSSH 也会处理两次）。
    include_stack: HashSet<PathBuf>,
}

impl ConfigParser<'_> {
    /// 解析一段配置文本；`depth` 是当前 Include 嵌套层数。
    fn parse_content(&mut self, content: &str, depth: usize) {
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = split_directive(line) else {
                continue;
            };
            self.apply_directive(key, value, depth);
        }
    }

    /// 处理一条指令；未知指令与历史行为一致，静默忽略。
    fn apply_directive(&mut self, key: &str, value: &str, depth: usize) {
        if key.eq_ignore_ascii_case("Host") {
            self.flush_host();
            for host in value.split_whitespace() {
                self.hosts.push(host.to_string());
            }
        } else if key.eq_ignore_ascii_case("HostName") {
            self.hostname = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("User") {
            self.user = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("Port") {
            if let Ok(port) = value.parse::<u16>() {
                self.port = Some(port);
            }
        } else if key.eq_ignore_ascii_case("IdentityFile") {
            let raw = value.trim_matches('"');
            self.identity = Some(expand_tilde(raw, self.home));
        } else if key.eq_ignore_ascii_case("Include") {
            self.parse_include(value, depth);
        }
    }

    /// 展开一条 `Include`：支持多模式、`~`、相对 `~/.ssh` 与末级通配。
    ///
    /// 单个文件读取失败只警告并跳过：主配置里残留一条指向已删除文件的
    /// Include 不该让整次导入失败——用户要的是其余可用会话。
    fn parse_include(&mut self, value: &str, depth: usize) {
        if depth >= MAX_INCLUDE_DEPTH {
            log::warn!("SSH 配置的 Include 嵌套超过 {MAX_INCLUDE_DEPTH} 层，已停止展开: {value}");
            return;
        }
        let base = self.home.map(|home| home.join(".ssh"));
        for token in split_tokens(value) {
            let expanded = expand_tilde(&token, self.home);
            let pattern = if expanded.is_absolute() {
                expanded
            } else if let Some(base) = &base {
                // OpenSSH 的相对路径基准是 ~/.ssh，不是包含它的文件所在目录。
                base.join(expanded)
            } else {
                log::warn!("SSH 配置的 Include 缺少家目录，无法解析相对路径，已忽略: {token}");
                continue;
            };
            let files = expand_glob(&pattern);
            if files.is_empty() {
                log::debug!("SSH 配置的 Include 没有匹配到文件: {}", pattern.display());
            }
            for path in files {
                let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                if !self.include_stack.insert(canonical.clone()) {
                    log::warn!("SSH 配置出现循环 Include，已跳过: {}", path.display());
                    continue;
                }
                match std::fs::read_to_string(&path) {
                    Ok(content) => self.parse_content(&content, depth + 1),
                    Err(error) => log::warn!(
                        "读取 SSH 配置 Include 失败，已跳过，文件: {}，原因: {error}",
                        path.display()
                    ),
                }
                self.include_stack.remove(&canonical);
            }
        }
    }

    /// 收尾当前 Host 块：为每个具体主机生成一个会话，随后清空块状态。
    fn flush_host(&mut self) {
        if self.hosts.is_empty() {
            return;
        }
        for host in std::mem::take(&mut self.hosts) {
            // 跳过通配符主机规则，如 *, *.*, ?
            if host.contains('*') || host.contains('?') {
                continue;
            }
            let target_host = self.hostname.clone().unwrap_or_else(|| host.clone());
            let target_port = self.port.unwrap_or(22);
            let target_user = self.user.clone().unwrap_or_else(|| {
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "root".to_string())
            });

            let auth = if let Some(key_path) = self.identity.clone() {
                AuthMethod::PrivateKey {
                    path: key_path,
                    cert_path: None,
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
                proxy_jump: None,
            });

            // 文件夹归类由导入流程统一创建，解析器只负责连接参数。
            self.sessions.push(Session::new(host, connection));
        }
        self.hostname = None;
        self.user = None;
        self.port = None;
        self.identity = None;
    }
}

/// 把一行配置拆成指令名与值；`Key value` 与 `Key=value` 两种写法都接受。
fn split_directive(line: &str) -> Option<(&str, &str)> {
    if let Some((key, value)) = line.split_once('=') {
        Some((key.trim(), value.trim()))
    } else {
        line.split_once(char::is_whitespace)
            .map(|(key, value)| (key.trim(), value.trim()))
    }
}

/// 按空白拆分 Include 的模式串，双引号内的空白不拆。
fn split_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in value.chars() {
        match ch {
            '"' => quoted = !quoted,
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 展开 `~` 与 `~/...`；无法展开时按原样返回。
fn expand_tilde(raw: &str, home: Option<&Path>) -> PathBuf {
    if raw == "~" {
        return home.map_or_else(|| PathBuf::from(raw), PathBuf::from);
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

/// 展开末级通配符；不含通配符时只返回真实存在的文件。
///
/// 结果排序后再返回，保证同一份配置每次导入得到相同的会话顺序。
fn expand_glob(pattern: &Path) -> Vec<PathBuf> {
    let Some(file_name) = pattern.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    if !file_name.contains(['*', '?', '[']) {
        return if pattern.is_file() {
            vec![pattern.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    let parent = pattern.parent().unwrap_or_else(|| Path::new("."));
    let Ok(glob) = Glob::new(file_name) else {
        log::warn!("SSH 配置的 Include 通配表达式非法，已忽略: {file_name}");
        return Vec::new();
    };
    let matcher = glob.compile_matcher();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matcher.is_match(name))
        })
        .collect();
    files.sort();
    files
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
        assert_eq!(dev.folder_id, None);
        if let Connection::Ssh(ssh) = &dev.connection {
            assert_eq!(ssh.host, "192.168.1.50");
            assert_eq!(ssh.username, "developer");
            assert_eq!(ssh.port, 2222);
            assert_eq!(
                ssh.auth,
                AuthMethod::PrivateKey {
                    path: PathBuf::from("/custom/home/.ssh/id_ed25519"),
                    cert_path: None,
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

    /// 建一个带 `config.d` 的临时家目录，返回 (守卫, 家目录路径)。
    fn temp_home(tag: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("临时目录应可创建");
        let home = dir.path().join(tag);
        std::fs::create_dir_all(home.join(".ssh").join("config.d")).expect("目录应可创建");
        (dir, home)
    }

    fn session_names(sessions: &[Session]) -> Vec<&str> {
        sessions
            .iter()
            .map(|session| session.name.as_str())
            .collect()
    }

    #[test]
    fn include_expands_globbed_files_in_sorted_order() {
        let (_guard, home) = temp_home("glob");
        let config_d = home.join(".ssh").join("config.d");
        std::fs::write(
            config_d.join("10-work.conf"),
            "Host work\n    HostName work.example\n    User w\n",
        )
        .unwrap();
        std::fs::write(
            config_d.join("20-lab.conf"),
            "Host lab\n    HostName lab.example\n    User l\n",
        )
        .unwrap();
        let sessions = parse_ssh_config_with_home(
            "Include config.d/*.conf\nHost tail\n    HostName tail.example\n",
            Some(&home),
        );
        // 相对路径按 ~/.ssh 解析；结果按文件名排序，主文件自己的 Host 最后。
        assert_eq!(session_names(&sessions), vec!["work", "lab", "tail"]);
        let Connection::Ssh(work) = &sessions[0].connection else {
            panic!("应为 SSH 连接");
        };
        assert_eq!(work.host, "work.example");
    }

    #[test]
    fn include_continues_the_current_host_block() {
        let (_guard, home) = temp_home("continue");
        std::fs::write(
            home.join(".ssh").join("config.d").join("port.conf"),
            "    Port 2200\n",
        )
        .unwrap();
        // 文本插入语义：Include 位于 Host 块内部，其内容属于同一个块。
        let sessions = parse_ssh_config_with_home(
            "Host merged\n    HostName merged.example\n    Include config.d/port.conf\n    User u\n",
            Some(&home),
        );
        assert_eq!(session_names(&sessions), vec!["merged"]);
        let Connection::Ssh(ssh) = &sessions[0].connection else {
            panic!("应为 SSH 连接");
        };
        assert_eq!(ssh.port, 2200);
        assert_eq!(ssh.username, "u");
    }

    #[test]
    fn include_cycle_terminates_and_keeps_hosts() {
        let (_guard, home) = temp_home("cycle");
        let config_d = home.join(".ssh").join("config.d");
        std::fs::write(
            config_d.join("a.conf"),
            "Include config.d/b.conf\nHost a-host\n    HostName a.example\n",
        )
        .unwrap();
        std::fs::write(config_d.join("b.conf"), "Include config.d/a.conf\n").unwrap();
        let sessions = parse_ssh_config_with_home("Include config.d/a.conf\n", Some(&home));
        // 环被切断，但两个入口文件各自的 Host 仍然可用。
        assert_eq!(session_names(&sessions), vec!["a-host"]);
    }

    #[test]
    fn include_missing_file_is_ignored() {
        let (_guard, home) = temp_home("missing");
        let sessions = parse_ssh_config_with_home(
            "Include config.d/not-there.conf\nHost kept\n    HostName kept.example\n",
            Some(&home),
        );
        assert_eq!(session_names(&sessions), vec!["kept"]);
    }

    #[test]
    fn include_accepts_tilde_and_quoted_paths() {
        let (_guard, home) = temp_home("quoted");
        let ssh = home.join(".ssh");
        std::fs::write(
            ssh.join("extra.conf"),
            "Host extra\n    HostName extra.example\n",
        )
        .unwrap();
        std::fs::write(
            ssh.join("with space.conf"),
            "Host spaced\n    HostName spaced.example\n",
        )
        .unwrap();
        // 一条 Include 写多个模式；带空格的路径用引号包起来。
        let sessions = parse_ssh_config_with_home(
            "Include ~/.ssh/extra.conf \"~/.ssh/with space.conf\"\n",
            Some(&home),
        );
        assert_eq!(session_names(&sessions), vec!["extra", "spaced"]);
    }
}
