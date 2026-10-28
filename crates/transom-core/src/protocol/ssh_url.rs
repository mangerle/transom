//! SSH 快速连接主机串与 `ssh://` URL 解析模块。
//!
//! 设计原理（Why）：Quick Connect 与桌面深链（浏览器点击 `ssh://` 链接）共用
//! 一套地址语法，两处若各写各的解析会导致行为分叉，因此收敛到本模块。
//! 不借助 `url` crate：SSH 地址只是 `[user@]host[:port]` 的窄语法，`url` 会
//! 对 `user@host:port` 这类无 scheme 输入产生歧义（把 `user@host` 当 scheme），
//! 手写解析反而语义精确、零新增依赖。
//!
//! 权衡（Trade-offs）：不支持 URL 路径、query 参数与 IPv6 Zone ID，换取解析
//! 规则的完全可控；这些扩展在终端连接场景中没有对应语义，将来需要时在本模块
//! 增量扩展即可，不影响调用方契约。

use crate::model::session::{AuthMethod, Connection, Session, SessionOptions, SshConnection};

/// 把未指定用户名/端口的地址补齐为可发起连接的 SSH 会话参数。
///
/// 与 OpenSSH 命令行行为对齐：缺省用户名取当前系统用户，缺省端口 22。
pub fn quick_connect_session(input: &str) -> Result<Session, String> {
    let target = parse_ssh_target(input)?;
    let username = match target.username {
        Some(user) => user,
        None => default_username(),
    };
    let name = if target.port == DEFAULT_SSH_PORT {
        format!("{username}@{}", target.host)
    } else {
        format!("{username}@{}:{}", target.host, target.port)
    };
    let connection = Connection::Ssh(SshConnection {
        host: target.host,
        port: target.port,
        username,
        auth: AuthMethod::Agent,
        options: SessionOptions::default(),
        proxy_jump: None,
    });
    Ok(Session::new(name, connection))
}

/// `ssh://` URL 与无 scheme 主机串的解析结果（用户名可为空，由调用方补齐）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub username: Option<String>,
    pub host: String,
    pub port: u16,
}

/// 默认 SSH 端口，补齐与展示（名称是否附端口）共用同一常量，避免魔数分叉。
pub const DEFAULT_SSH_PORT: u16 = 22;

/// 深链与快速连接识别的 URL scheme（小写字面量，比较处一律大小写不敏感）。
const SSH_SCHEME: &str = "ssh://";

/// 解析 `[user@]host[:port]` 或 `ssh://[user@]host[:port]`，返回三段原始值。
///
/// 规则：
/// - 带 scheme 前缀时按 URL 语义解析（用户名 percent 解码，忽略 query/fragment）；
/// - 不带 scheme 时禁止出现空白字符，避免把搜索词误认成主机（快速打开面板
///   同时承担搜索职责）；
/// - IPv6 字面量必须写作 `[::1]` 或 `ssh://[::1]:22`，防止冒号与端口分隔符歧义。
pub fn parse_ssh_target(input: &str) -> Result<SshTarget, String> {
    let input = input.trim();
    // scheme 判定与剥离共用 is_ssh_url 的大小写不敏感规则，避免桌面深链
    // （大小写随启动器）与快速连接输入框两处行为分叉。
    let scheme = is_ssh_url(input);
    let body = match scheme {
        true => &input[SSH_SCHEME.len()..],
        false => {
            if input.is_empty() || input.chars().any(char::is_whitespace) {
                return Err("快速连接地址不能为空或包含空白字符".to_string());
            }
            input
        }
    };
    // query 与 fragment 没有连接语义（浏览器/工具拼接的脏数据），直接剥离。
    let body = body
        .split(['?', '#'])
        .next()
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| format!("SSH 地址缺少主机部分，原始输入: {input}"))?;

    let (username, rest) = split_username(body, scheme)?;
    let (host, port) = split_host_port(rest)?;
    if host.is_empty() {
        return Err(format!("SSH 地址缺少主机部分，原始输入: {input}"));
    }
    Ok(SshTarget {
        username,
        host,
        port: port.unwrap_or(DEFAULT_SSH_PORT),
    })
}

/// 拆出 `user@` 前缀；主机部分从剩余串继续解析。
///
/// 只按最后一个 '@' 切分：这样 `user@[fe80::1]` 的主机段保持完整。用户名
/// 含 '[' 说明 '@' 出现在 IPv6 括号之后（非法写法），空用户名同样拒绝。
fn split_username(body: &str, scheme: bool) -> Result<(Option<String>, &str), String> {
    match body.rsplit_once('@') {
        Some((user, rest)) => {
            if user.is_empty() || user.contains('[') {
                return Err(format!("SSH 地址用户名部分非法，原始输入: {body}"));
            }
            // percent 解码只在 scheme 形式下有意义；裸 host 串保持字面量。
            let user = if scheme {
                percent_decode(user)
            } else {
                user.to_string()
            };
            Ok((Some(user), rest))
        }
        None => Ok((None, body)),
    }
}

/// 拆出主机与端口，返回 `(host, Option<port>)`。
fn split_host_port(rest: &str) -> Result<(String, Option<u16>), String> {
    if let Some(bracketed) = rest.strip_prefix('[') {
        let (host, tail) = bracketed
            .split_once(']')
            .ok_or_else(|| format!("IPv6 主机缺少右方括号，原始输入: {rest}"))?;
        return match tail {
            "" => Ok((format!("[{host}]"), None)),
            port_text => parse_port(port_text.strip_prefix(':').unwrap_or(port_text), rest)
                .map(|port| (format!("[{host}]"), Some(port))),
        };
    }
    // 带 scheme 的裸 IPv6（ssh://fe80::1）不符合 RFC 规范形式（必须加方括号），
    // 且与 host:port 语法歧义；无 scheme 时同样无法区分，统一显式拒绝。
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port_text)) => {
            if host.contains(':') {
                return Err(format!(
                    "IPv6 地址必须写作 [地址] 或 ssh://[地址]:端口 形式，原始输入: {rest}"
                ));
            }
            (host, Some(parse_port(port_text, rest)?))
        }
        None => (rest, None),
    };
    validate_host(host, rest)?;
    Ok((host.to_string(), port))
}

/// 端口只接受纯数字且在 u16 范围内，避免 `host:name` 之类的误写静默变 22。
fn parse_port(text: &str, context: &str) -> Result<u16, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("SSH 地址端口部分非法，原始输入: {context}"));
    }
    text.parse::<u16>()
        .map_err(|_| format!("SSH 地址端口超出范围，原始输入: {context}"))
}

/// 主机字面量校验：RFC 允许的字符集（字母、数字、连字符、点、下划线），
/// 拒绝空白与控制字符——主机名可能来自浏览器 URL，必须按不可信输入对待。
fn validate_host(host: &str, context: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err(format!("SSH 地址缺少主机部分，原始输入: {context}"));
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'%'))
    {
        return Err(format!("SSH 地址主机部分含非法字符，原始输入: {context}"));
    }
    Ok(())
}

/// 缺省用户名：对齐 OpenSSH 客户端行为，取当前系统用户。
///
/// `USER` 环境变量在少数精简环境（systemd 拉起、容器）可能缺失，此时回落
/// `whoami` 不可用，用 `root` 兜底并交由连接阶段的用户名询问修正。
fn default_username() -> String {
    std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("USERNAME").ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "root".to_string())
}

/// URL percent 解码（只需处理用户名中的 `%XX`，其余原样保留）。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && is_hex(bytes[i + 1]) && is_hex(bytes[i + 2]) {
            // 不变性：i+1、i+2 已通过 is_hex 校验，from_str_radix 必然成功。
            let value = u8::from_str_radix(&input[i + 1..i + 3], 16)
                .expect("已通过 is_hex 校验的两位十六进制序列必然可解析");
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// 判断命令行参数是否为 `ssh://` 深链地址（大小写不敏感）。
///
/// 桌面环境把 URL 作为 argv 交给应用时不做规范化，可能带任意大小写与前导
/// 空白；非此形态的参数（文件路径、启动器附加的选项）一律忽略，避免把
/// 无关参数当成主机连接。
pub fn is_ssh_url(arg: &str) -> bool {
    let trimmed = arg.trim();
    trimmed.len() >= SSH_SCHEME.len()
        && trimmed.as_bytes()[..SSH_SCHEME.len()].eq_ignore_ascii_case(SSH_SCHEME.as_bytes())
}

/// 判断输入是否更像"主机地址"而不是"会话搜索词"（快速打开面板的虚拟项门槛）。
///
/// 门槛刻意保守：只有带 `ssh://` 前缀、含 `@`、或整体呈 `host[:port]` 形态
/// （字符集合法且至少含点/冒号）才认定为地址，避免 "prod" 这类会话名简称
/// 触发无关的"直连"项。
pub fn looks_like_host(input: &str) -> bool {
    let input = input.trim();
    if input.is_empty() || input.chars().any(char::is_whitespace) {
        return false;
    }
    if is_ssh_url(input) || input.contains('@') {
        return true;
    }
    // 方括号包裹的 IPv6 字面量天然是地址，无需再满足点号门槛。
    if input.starts_with('[') && input.ends_with(']') {
        return true;
    }
    let host_part = input.split(':').next().unwrap_or(input);
    host_part.contains('.')
        && host_part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'%'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(input: &str) -> SshTarget {
        parse_ssh_target(input).unwrap_or_else(|err| panic!("解析 {input} 失败: {err}"))
    }

    #[test]
    fn bare_host_uses_defaults() {
        let parsed = target("example.com");
        assert_eq!(parsed.username, None);
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 22);
    }

    #[test]
    fn bare_host_with_user_and_port() {
        let parsed = target("deploy@10.0.0.8:2222");
        assert_eq!(parsed.username.as_deref(), Some("deploy"));
        assert_eq!(parsed.host, "10.0.0.8");
        assert_eq!(parsed.port, 2222);
    }

    #[test]
    fn scheme_forms_parse_like_bare_forms() {
        assert_eq!(target("ssh://example.com"), target("example.com"));
        assert_eq!(
            target("ssh://deploy@host.example:2200"),
            target("deploy@host.example:2200")
        );
    }

    #[test]
    fn scheme_ignores_query_and_fragment() {
        // 浏览器/外部工具可能在深链 URL 后拼接参数，不应污染主机解析。
        let parsed = target("ssh://user@host.example:2222?from=web#top");
        assert_eq!(parsed.host, "host.example");
        assert_eq!(parsed.port, 2222);
        assert_eq!(parsed.username.as_deref(), Some("user"));
    }

    #[test]
    fn scheme_percent_decodes_username() {
        let parsed = target("ssh://my%40user@host.example");
        assert_eq!(parsed.username.as_deref(), Some("my@user"));
    }

    #[test]
    fn ipv6_requires_brackets_and_keeps_them_for_connect() {
        let parsed = target("ssh://root@[fe80::1]:2222");
        assert_eq!(parsed.host, "[fe80::1]");
        assert_eq!(parsed.port, 2222);
        let bare = target("[::1]");
        assert_eq!(bare.host, "[::1]");
        assert_eq!(bare.port, 22);
    }

    #[test]
    fn bare_ipv6_without_brackets_is_rejected() {
        // 裸冒号与 host:port 语法歧义，必须显式报错而不是猜端口。
        assert!(parse_ssh_target("fe80::1").is_err());
        assert!(parse_ssh_target("ssh://fe80::1").is_err());
    }

    #[test]
    fn invalid_ports_and_hosts_are_rejected() {
        assert!(parse_ssh_target("host:").is_err());
        assert!(parse_ssh_target("host:abc").is_err());
        assert!(parse_ssh_target("host:70000").is_err());
        assert!(parse_ssh_target("hos t").is_err());
        assert!(parse_ssh_target("").is_err());
        assert!(parse_ssh_target("ssh://").is_err());
        assert!(parse_ssh_target("@host").is_err());
        assert!(parse_ssh_target("user@").is_err());
    }

    #[test]
    fn session_name_includes_non_default_port() {
        let session = quick_connect_session("deploy@10.0.0.8:2222").unwrap();
        assert_eq!(session.name, "deploy@10.0.0.8:2222");
        if let Connection::Ssh(ssh) = &session.connection {
            assert_eq!(ssh.host, "10.0.0.8");
            assert_eq!(ssh.username, "deploy");
            assert_eq!(ssh.port, 2222);
            assert!(matches!(ssh.auth, AuthMethod::Agent));
        } else {
            panic!("应生成 SSH 连接");
        }
    }

    /// 深链 URL 的大小写由桌面启动器决定，scheme 判定必须不敏感；
    /// 但只有出现在开头的 ssh:// 才算地址，句中出现的不能误判。
    #[test]
    fn scheme_detection_is_case_insensitive_and_anchored() {
        assert!(is_ssh_url("ssh://host.example"));
        assert!(is_ssh_url("SSH://host.example"));
        assert!(is_ssh_url("  ssh://host.example"));
        assert!(!is_ssh_url("host.example"));
        assert!(!is_ssh_url("not-ssh://host"));
        assert!(!is_ssh_url("ssh:/host"));

        // scheme 大小写不敏感必须一路贯通到解析结果。
        assert_eq!(
            target("SSH://deploy@host.example:2222"),
            target("deploy@host.example:2222")
        );
        assert!(looks_like_host("SSH://host.example"));
    }

    #[test]
    fn looks_like_host_gate_is_conservative() {
        assert!(looks_like_host("10.0.0.1"));
        assert!(looks_like_host("host.example.com"));
        assert!(looks_like_host("user@host"));
        assert!(looks_like_host("ssh://host.example"));
        assert!(looks_like_host("[::1]"));
        assert!(!looks_like_host("prod"));
        assert!(!looks_like_host("bash"));
        assert!(!looks_like_host("two words"));
        assert!(!looks_like_host(""));
    }
}
