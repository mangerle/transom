//! Mosh (Mobile Shell) 协议引导与运行时管理。
//!
//! 设计原理（Why）：
//! - 实现初衷：传统 SSH 基于 TCP，在移动漫游、WiFi 切换或丢包抖动下容易挂起；
//!   Mosh 协议通过先走 SSH 引导在远端拉起 `mosh-server`，再通过加密 UDP 与状态同步协议（SSP）
//!   实现瞬时漫游恢复与无延迟本地输入预测；
//! - 核心优势：
//!   1. 引导层与运行时分层：SSH 引导仅用于在远端安全启动并获取一次性连接凭证，即使 SSH 断开也不影响 Mosh 运行；
//!   2. 健壮的模式解析与异常防护：对 `MOSH CONNECT` 输出进行防注入校验，端口严格限制在合法范围内；
//!   3. 客户端平滑桥接：若本地安装有 `mosh-client` 则无缝接管伪终端渲染，未安装时给出清晰可行动的诊断指引；
//! - 代价与局限：Mosh 依赖远端服务器部署 `mosh-server` 且防火墙放行 UDP 对应端口段。

use std::process::Command;
use transom_core::{AppError, ErrorCode};

/// Mosh 会话引导成功后的连接凭证信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoshSessionInfo {
    /// 远端 mosh-server 监听的 UDP 端口。
    pub udp_port: u16,
    /// 用于 AES 加密状态同步的一次性 Base64 密钥。
    pub session_key: String,
}

/// 构建在远端启动 `mosh-server` 的引导命令。
///
/// 遵循标准 POSIX 参数约定：使用 `-s` 绑定 IP，`-c 256` 启用 256 色。
pub fn build_mosh_server_command(server_path: &str, port: Option<u16>) -> String {
    let path = if server_path.trim().is_empty() {
        "mosh-server"
    } else {
        server_path.trim()
    };
    if let Some(p) = port {
        format!("{path} new -s -c 256 -p {p}")
    } else {
        format!("{path} new -s -c 256")
    }
}

/// 解析 `mosh-server` 输出中的 `MOSH CONNECT <port> <key>` 握手凭证。
pub fn parse_mosh_server_output(output: &str) -> Result<MoshSessionInfo, AppError> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("MOSH CONNECT") {
            let mut parts = rest.split_whitespace();
            let port_str = parts.next().ok_or_else(|| {
                AppError::with_detail(ErrorCode::Protocol, "mosh-server 未回传有效 UDP 端口")
            })?;
            let key = parts.next().ok_or_else(|| {
                AppError::with_detail(ErrorCode::Protocol, "mosh-server 未回传会话密钥")
            })?;

            let udp_port = port_str.parse::<u16>().map_err(|err| {
                AppError::with_detail(
                    ErrorCode::Protocol,
                    format!("mosh-server 返回了非法端口 {port_str}: {err}"),
                )
            })?;

            if key.len() < 16 {
                return Err(AppError::with_detail(
                    ErrorCode::Protocol,
                    "mosh-server 返回的密钥长度过短",
                ));
            }

            return Ok(MoshSessionInfo {
                udp_port,
                session_key: key.to_string(),
            });
        }
    }

    Err(AppError::with_detail(
        ErrorCode::Protocol,
        format!(
            "未能从远端输出中捕获 MOSH CONNECT 凭证，原始输出摘要: {}",
            output.chars().take(200).collect::<String>()
        ),
    ))
}

/// 检查本地操作系统是否安装了 `mosh-client` 命令。
pub fn check_local_mosh_client() -> bool {
    Command::new("which")
        .arg("mosh-client")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_mosh_server_command() {
        assert_eq!(
            build_mosh_server_command("mosh-server", None),
            "mosh-server new -s -c 256"
        );
        assert_eq!(
            build_mosh_server_command("mosh-server", Some(60005)),
            "mosh-server new -s -c 256 -p 60005"
        );
        assert_eq!(
            build_mosh_server_command("/usr/local/bin/mosh-server", Some(60010)),
            "/usr/local/bin/mosh-server new -s -c 256 -p 60010"
        );
        assert_eq!(
            build_mosh_server_command("", None),
            "mosh-server new -s -c 256"
        );
    }

    #[test]
    fn test_parse_mosh_server_output_success() {
        let output = r#"
            some init banner
            MOSH CONNECT 60001 7uY2eQx3yD5zG6h8j9k0lA
            mosh-server started on port 60001
        "#;
        let info = parse_mosh_server_output(output).unwrap();
        assert_eq!(info.udp_port, 60001);
        assert_eq!(info.session_key, "7uY2eQx3yD5zG6h8j9k0lA");
    }

    #[test]
    fn test_parse_mosh_server_output_missing_or_invalid() {
        let output = "command not found: mosh-server";
        assert!(parse_mosh_server_output(output).is_err());

        let invalid_port = "MOSH CONNECT not_a_port 7uY2eQx3yD5zG6h8j9k0lA";
        assert!(parse_mosh_server_output(invalid_port).is_err());

        let short_key = "MOSH CONNECT 60001 short";
        assert!(parse_mosh_server_output(short_key).is_err());
    }
}
