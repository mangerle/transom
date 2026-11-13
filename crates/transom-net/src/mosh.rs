//! Mosh 引导与本地客户端探测。
//!
//! 设计原理（Why）：Mosh 依赖远端 `mosh-server` 与本地 `mosh-client`。
//! Linux/macOS 可直接执行 PATH 中的客户端；Windows 原生通常没有 Mosh，
//! 因此通过 WSL 启动 `mosh-client`，再用 `WSLENV` 传递一次性 `MOSH_KEY`。
//!
//! 安全约束：会话密钥只进入环境变量，禁止拼入命令行；命令与参数均以 argv
//! 传递，不经过 Shell 字符串解释。

use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::Duration;
use transom_core::{AppError, ErrorCode};

/// 本地 mosh-client 的探测超时；WSL 冷启动可能略慢，但不能无限等待。
#[cfg(windows)]
const CLIENT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Mosh 会话引导成功后的连接凭证信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoshSessionInfo {
    /// 远端 mosh-server 监听的 UDP 端口。
    pub udp_port: u16,
    /// 用于 AES 加密状态同步的一次性 Base64 密钥。
    pub session_key: String,
}

/// 本地 Mosh 客户端启动形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoshClientCommand {
    pub program: String,
    pub prefix: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// 构造远端 mosh-server 启动命令。
pub fn build_mosh_server_command(path: &str, port: Option<u16>) -> String {
    let path = path.trim();
    let path = if path.is_empty() { "mosh-server" } else { path };
    match port {
        Some(port) => format!("{path} new -s -c 256 -p {port}"),
        None => format!("{path} new -s -c 256"),
    }
}

/// 从 mosh-server 输出解析 `MOSH CONNECT <port> <key>`。
pub fn parse_mosh_server_output(output: &str) -> Result<MoshSessionInfo, AppError> {
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("MOSH") || parts.next() != Some("CONNECT") {
            continue;
        }
        let port_str = parts.next().ok_or_else(|| {
            AppError::with_detail(ErrorCode::Protocol, "mosh-server 未回传连接端口")
        })?;
        let key = parts.next().ok_or_else(|| {
            AppError::with_detail(ErrorCode::Protocol, "mosh-server 未回传会话密钥")
        })?;
        let udp_port = port_str.parse::<u16>().map_err(|error| {
            AppError::with_detail(
                ErrorCode::Protocol,
                format!("mosh-server 返回了非法端口 {port_str}: {error}"),
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
    Err(AppError::with_detail(
        ErrorCode::Protocol,
        format!(
            "未能从远端输出中捕获 MOSH CONNECT 凭证，原始输出摘要: {}",
            output.chars().take(200).collect::<String>()
        ),
    ))
}

/// 解析当前平台的本地 Mosh 客户端。
///
/// Linux/macOS 查找 PATH 中的 `mosh-client`；Windows 探测 WSL 内是否安装
/// `mosh-client`，并可执行时返回带 WSLENV 环境传递的启动形态。
///
/// # Errors
/// 本地无 Mosh 客户端、WSL 不可用或探测超时时返回带平台上下文的配置错误。
pub async fn local_mosh_client() -> Result<MoshClientCommand, AppError> {
    #[cfg(not(windows))]
    {
        if find_in_path("mosh-client").is_some() {
            Ok(MoshClientCommand {
                program: "mosh-client".to_string(),
                prefix: Vec::new(),
                env: Vec::new(),
            })
        } else {
            Err(AppError::with_detail(
                ErrorCode::Configuration,
                "未找到 mosh-client，请先通过系统包管理器安装 mosh",
            ))
        }
    }
    #[cfg(windows)]
    {
        windows_wsl_mosh_client().await
    }
}

#[cfg(windows)]
async fn windows_wsl_mosh_client() -> Result<MoshClientCommand, AppError> {
    let Some(wsl) = find_in_path("wsl.exe") else {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "Windows 原生不支持 mosh-client；请安装 WSL 并在发行版中安装 mosh",
        ));
    };
    let mut command = tokio::process::Command::new(&wsl);
    command
        .args([
            "--exec",
            "sh",
            "-lc",
            "command -v mosh-client >/dev/null 2>&1",
        ])
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let available = match tokio::time::timeout(CLIENT_PROBE_TIMEOUT, command.status()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(error)) => {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "探测 WSL mosh-client 失败，程序: {}，原因: {error}",
                    wsl.display()
                ),
            ));
        }
        Err(_) => {
            return Err(AppError::with_detail(
                ErrorCode::Timeout,
                format!(
                    "探测 WSL mosh-client 超时，程序: {}，超时: {} 秒",
                    wsl.display(),
                    CLIENT_PROBE_TIMEOUT.as_secs()
                ),
            ));
        }
    };
    if !available {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "WSL 中未找到 mosh-client；请在默认 WSL 发行版中安装 mosh",
        ));
    }
    Ok(MoshClientCommand {
        program: wsl.to_string_lossy().into_owned(),
        prefix: vec!["--exec".to_string(), "mosh-client".to_string()],
        env: vec![("WSLENV".to_string(), "MOSH_KEY".to_string())],
    })
}

/// 在 PATH 中查找可执行文件。
fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && path
                .metadata()
                .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_mosh_server_command, parse_mosh_server_output};

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
        assert!(parse_mosh_server_output("command not found: mosh-server").is_err());
        assert!(
            parse_mosh_server_output("MOSH CONNECT not_a_port 7uY2eQx3yD5zG6h8j9k0lA").is_err()
        );
    }
}
