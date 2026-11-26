//! 新建 SSH 会话的批量主机解析与多会话构建逻辑。
//!
//! 设计原理（Why）：
//! - 与单会话 `builder.rs` 分离，避免单文件超过 500 行；方法仍归属 [`SessionForm`]。
//! - 用户在批量模式下填写多个不同 IP/主机但共享同一套端口、分组、用户名/认证
//!   与网络参数时，为每个目标生成独立的 `Session` 与 `SessionId`，便于后续单独调整。

use super::types::{BuiltSession, ConnectionChoice, SessionForm};
use gpui_kit::Context;
use transom_core::{Connection, Session};

/// 批量主机行解析结果：`(可选自定义名称, 主机地址, 端口)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BatchHostTarget {
    pub custom_name: Option<String>,
    pub host: String,
    pub port: u16,
}

impl SessionForm {
    /// 批量模式下构建多个 SSH 会话；复用当前表单的认证、分组、代理与终端配置。
    ///
    /// # Errors
    /// 批量列表为空、端口非法、用户名/私钥缺失或网络参数校验失败时返回本地化文案。
    pub(super) fn build_batch_sessions(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<Vec<BuiltSession>, String> {
        if self.connection != ConnectionChoice::Ssh {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        let default_port: u16 = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_port").to_string())?;
        if default_port == 0 {
            return Err(rust_i18n::t!("form.error_port").to_string());
        }

        let raw_hosts = self.batch_hosts.read(cx).value().to_string();
        let targets = parse_batch_host_lines(&raw_hosts, default_port)?;
        self.build_batch_from_targets(&targets, cx)
    }

    fn build_batch_from_targets(
        &self,
        targets: &[BatchHostTarget],
        cx: &mut Context<impl Sized>,
    ) -> Result<Vec<BuiltSession>, String> {
        let name_prefix = self.name.read(cx).value().trim().to_string();
        let template_ssh = self.build_ssh_template(cx)?;
        let folder_path = {
            let path = self.folder_path.read(cx).value().trim().to_string();
            (!path.is_empty()).then_some(path)
        };
        let startup =
            transom_core::StartupCommands::from_multiline(&self.startup_commands.read(cx).value());
        let proxy = self.build_proxy_public(cx)?;

        let mut built_list = Vec::with_capacity(targets.len());
        for target in targets {
            let session_name = match &target.custom_name {
                Some(custom) => custom.clone(),
                None if !name_prefix.is_empty() => format!("{name_prefix}-{}", target.host),
                None => target.host.clone(),
            };
            let mut ssh = template_ssh.connection.clone();
            if let Connection::Ssh(ref mut ssh_conn) = ssh {
                ssh_conn.host = target.host.clone();
                ssh_conn.port = target.port;
            }
            let mut session = Session::new(session_name, ssh);
            session.folder_id = None;
            session.startup = startup.clone();
            session.identity_id = self.identity_choice;
            session.proxy = proxy.clone();
            session.icon = self.icon;
            session.color = self.color;
            session.block_view = self.block_view;
            session.forwards = self.forwards.clone();
            session.terminal = transom_core::TerminalProfile::default();
            session.touch();

            built_list.push(BuiltSession {
                session,
                folder_path: folder_path.clone(),
                password: template_ssh.password.clone(),
                key_passphrase: template_ssh.key_passphrase.clone(),
            });
        }
        Ok(built_list)
    }
}

/// 解析多行批量主机清单。
///
/// 支持每行格式：
/// - `192.168.1.10`
/// - `192.168.1.10:2222`
/// - `[::1]:2222`
/// - `web-01 192.168.1.10` 或 `web-01 192.168.1.10:2222`
pub(super) fn parse_batch_host_lines(
    text: &str,
    default_port: u16,
) -> Result<Vec<BatchHostTarget>, String> {
    let mut targets = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let (custom_name, endpoint) = match tokens.as_slice() {
            [only] => (None, *only),
            [name, endpoint, ..] => (Some((*name).to_string()), *endpoint),
            [] => continue,
        };
        let (host, port) = split_host_port(endpoint, default_port)?;
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        targets.push(BatchHostTarget {
            custom_name,
            host,
            port,
        });
    }
    if targets.is_empty() {
        return Err(rust_i18n::t!("form.error_batch_hosts_empty").to_string());
    }
    Ok(targets)
}

/// 拆分 `host[:port]`，兼容 `[ipv6]:port` 与纯 IPv6 地址。
fn split_host_port(endpoint: &str, default_port: u16) -> Result<(String, u16), String> {
    let trimmed = endpoint.trim();
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some((ipv6, after_bracket)) = rest.split_once(']')
    {
        if let Some(port_str) = after_bracket.strip_prefix(':') {
            let port = port_str
                .parse::<u16>()
                .ok()
                .filter(|&p| p > 0)
                .ok_or_else(|| rust_i18n::t!("form.error_port").to_string())?;
            return Ok((ipv6.to_string(), port));
        }
        return Ok((ipv6.to_string(), default_port));
    }

    // 仅当只有一个冒号时才将后半段视为端口，避免把裸 IPv6 地址拆碎。
    if trimmed.matches(':').count() == 1
        && let Some((host, port_str)) = trimmed.rsplit_once(':')
    {
        let port = port_str
            .parse::<u16>()
            .ok()
            .filter(|&p| p > 0)
            .ok_or_else(|| rust_i18n::t!("form.error_port").to_string())?;
        return Ok((host.trim().to_string(), port));
    }

    Ok((trimmed.to_string(), default_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_batch_host_lines_variants() {
        let input = "\
# 注释行应跳过
192.168.1.10
192.168.1.11:2222
web-01 10.0.0.5
db-01 10.0.0.6:33022
[::1]:2200
";
        let targets = parse_batch_host_lines(input, 22).expect("应成功解析批量主机");
        assert_eq!(targets.len(), 5);
        assert_eq!(
            targets[0],
            BatchHostTarget {
                custom_name: None,
                host: "192.168.1.10".to_string(),
                port: 22
            }
        );
        assert_eq!(
            targets[1],
            BatchHostTarget {
                custom_name: None,
                host: "192.168.1.11".to_string(),
                port: 2222
            }
        );
        assert_eq!(
            targets[2],
            BatchHostTarget {
                custom_name: Some("web-01".to_string()),
                host: "10.0.0.5".to_string(),
                port: 22
            }
        );
        assert_eq!(
            targets[3],
            BatchHostTarget {
                custom_name: Some("db-01".to_string()),
                host: "10.0.0.6".to_string(),
                port: 33022
            }
        );
        assert_eq!(
            targets[4],
            BatchHostTarget {
                custom_name: None,
                host: "::1".to_string(),
                port: 2200
            }
        );
    }
}
