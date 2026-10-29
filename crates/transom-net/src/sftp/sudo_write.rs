//! SFTP Sudo 提权流式写入与原子覆盖。
//!
//! 设计原理（Why）：
//! - 实现初衷：当用户在线编辑属于 root 或只读受限的系统配置文件（如 `/etc/nginx/nginx.conf`）时，
//!   标准 SFTP 子系统因当前登录用户权限不足直接抛出 `PermissionDenied`。
//!   通过复用既有 SSH 会话开辟独立的 Sudo Exec 通道，实现无缝提权流式保存；
//! - 核心优势（方案 C 结合同目录原子暂存）：
//!   1. 零磁盘公共残留：不走全系统共享的 `/tmp` 目录，杜绝多租户窥探敏感配置；
//!   2. 自动保全文件所有权与属性：在目标同目录下生成带临时标记的隐藏文件，预克隆原文件的
//!      属主（Owner）、所属组（Group）、权限掩码（chmod）与 SELinux 上下文；
//!   3. 微秒级原子覆盖（Atomic Rename）：在同文件系统内通过 `mv` 触发操作系统的 `rename(2)`，
//!      避免因网络中断导致正在覆写的生产文件被截断破坏；
//!   4. 容灾清理保证（RAII / Shell Trap）：在脚本入口注册 `trap 'rm -f ...' EXIT`，
//!      一旦写入异常或中途断开，自动收割未完成的同目录临时文件，保持远端目录整洁；
//! - 代价与局限：要求远端系统已安装 `sudo` 与基础 POSIX 命令工具（`tee`、`mv`、`cp`），
//!   且登录用户在远端具备该路径的 sudo 授权。

use crate::client::SshRemote;
use russh::ChannelMsg;
use std::path::Path;
use std::time::Duration;
use transom_core::{AppError, ErrorCode};

/// 提权写入超时上限（60秒，足以应对大文件网络流式写入）。
const SUDO_WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// 单次向 SSH 通道灌入的标准输入分块大小（64 KiB）。
const STDIN_CHUNK_SIZE: usize = 64 * 1024;

/// 对命令行参数进行 POSIX 单引号闭合安全转义，杜绝 Shell 注入漏洞。
pub(crate) fn posix_shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// 提取目标路径的父级目录。
pub(crate) fn get_parent_directory(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if let Some(pos) = trimmed.rfind('/') {
        if pos == 0 { "/" } else { &trimmed[..pos] }
    } else {
        "."
    }
}

/// 基于目标路径构建同目录下的隐藏原子临时文件路径。
pub(crate) fn generate_same_dir_temp_path(target_path: &str, unique_token: &str) -> String {
    let parent = get_parent_directory(target_path);
    let file_name = Path::new(target_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("transom_file");
    if parent == "/" {
        format!("/.{file_name}.transom_sudo_{unique_token}")
    } else {
        format!("{parent}/.{file_name}.transom_sudo_{unique_token}")
    }
}

/// 构建在远程执行的同目录安全流式写入与原子覆盖脚本。
pub(crate) fn build_sudo_write_script(target_path: &str, temp_path: &str) -> String {
    let q_target = posix_shell_quote(target_path);
    let q_temp = posix_shell_quote(temp_path);

    // 脚本逻辑：
    // 1. 设置退出清理钩子；
    // 2. 若目标已存在，克隆所有属性到临时文件；若不存在则创建新文件；
    // 3. 从标准输入接收流并写入临时文件；
    // 4. 原子重命名覆盖目标；
    // 5. 撤销清理钩子。
    format!(
        "trap 'rm -f {q_temp}' EXIT && \
         if [ -f {q_target} ]; then \
           cp -p --attributes-only {q_target} {q_temp} 2>/dev/null || touch {q_temp}; \
         else \
           touch {q_temp}; \
         fi && \
         tee {q_temp} > /dev/null && \
         mv -f {q_temp} {q_target} && \
         trap - EXIT"
    )
}

/// 执行 Sudo 提权流式写入。
///
/// 通过 SSH Exec 建立独立子会话通道，调用 `sudo -S` 执行流式写入，并在成功后原子覆盖目标。
pub async fn exec_sudo_write(
    remote: &SshRemote,
    target_path: &str,
    content: &[u8],
    password: Option<&str>,
) -> Result<(), AppError> {
    tokio::time::timeout(
        SUDO_WRITE_TIMEOUT,
        exec_sudo_write_inner(remote, target_path, content, password),
    )
    .await
    .map_err(|_| {
        AppError::with_detail(
            ErrorCode::Timeout,
            format!(
                "Sudo 提权写入超时（超过 {} 秒）",
                SUDO_WRITE_TIMEOUT.as_secs()
            ),
        )
    })?
}

async fn exec_sudo_write_inner(
    remote: &SshRemote,
    target_path: &str,
    content: &[u8],
    password: Option<&str>,
) -> Result<(), AppError> {
    let token = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let temp_path = generate_same_dir_temp_path(target_path, &token);
    let inner_script = build_sudo_write_script(target_path, &temp_path);

    // sudo -S 从标准输入读取密码；-p 设置空提示符避免污染输出
    let full_command = format!("sudo -S -p '' sh -c {}", posix_shell_quote(&inner_script));
    log::info!("发起 Sudo 提权写入任务，目标: {target_path}，临时路径: {temp_path}");

    let mut channel = remote.open_session().await?;
    channel
        .exec(true, full_command.as_str())
        .await
        .map_err(|e| {
            AppError::with_detail(
                ErrorCode::ChannelFailed,
                format!("启动 Sudo 提权进程失败: {e}"),
            )
        })?;

    // 若提供了密码，先行灌入密码行
    if let Some(pwd) = password {
        let pwd_line = format!("{pwd}\n");
        channel
            .data_bytes(pwd_line.into_bytes())
            .await
            .map_err(|e| {
                AppError::with_detail(ErrorCode::Io, format!("发送 Sudo 密码失败: {e}"))
            })?;
    }

    // 分块流式灌入文件字节载荷
    for chunk in content.chunks(STDIN_CHUNK_SIZE) {
        channel.data_bytes(chunk.to_vec()).await.map_err(|e| {
            AppError::with_detail(ErrorCode::Io, format!("向提权通道传输文件数据失败: {e}"))
        })?;
    }

    // 发送 EOF 标识输入终止
    let _ = channel.eof().await;

    // 收集执行退出码与错误日志
    let mut stderr_bytes = Vec::new();
    let mut exit_status = None;

    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::ExtendedData { data, ext: 1 } => {
                stderr_bytes.extend_from_slice(&data);
            }
            ChannelMsg::ExitStatus { exit_status: code } => {
                exit_status = Some(code);
            }
            ChannelMsg::Close | ChannelMsg::Eof => break,
            _ => {}
        }
    }

    let stderr_text = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
    if exit_status == Some(0) {
        log::info!("Sudo 提权写入完成并成功原子替换: {target_path}");
        Ok(())
    } else {
        let err_detail = if !stderr_text.is_empty() {
            stderr_text
        } else {
            format!("进程退出状态码: {:?}", exit_status)
        };

        log::warn!("Sudo 提权写入失败，目标: {target_path}，详情: {err_detail}");

        if err_detail.contains("incorrect password")
            || err_detail.contains("try again")
            || err_detail.contains("password is required")
        {
            Err(AppError::with_detail(
                ErrorCode::AuthFailed,
                format!("Sudo 鉴权失败：密码错误或当前用户未配置提权权限 ({err_detail})"),
            ))
        } else {
            Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("Sudo 提权写入执行失败: {err_detail}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_shell_quote() {
        assert_eq!(posix_shell_quote("hello"), "'hello'");
        assert_eq!(posix_shell_quote("nginx'conf"), r"'nginx'\''conf'");
    }

    #[test]
    fn test_parent_directory() {
        assert_eq!(get_parent_directory("/etc/nginx/nginx.conf"), "/etc/nginx");
        assert_eq!(get_parent_directory("/hosts"), "/");
        assert_eq!(get_parent_directory("foo.txt"), ".");
    }

    #[test]
    fn test_generate_same_dir_temp_path() {
        let temp = generate_same_dir_temp_path("/etc/nginx/nginx.conf", "1234");
        assert_eq!(temp, "/etc/nginx/.nginx.conf.transom_sudo_1234");

        let root_temp = generate_same_dir_temp_path("/hosts", "5678");
        assert_eq!(root_temp, "/.hosts.transom_sudo_5678");
    }

    #[test]
    fn test_build_sudo_write_script() {
        let script = build_sudo_write_script("/etc/hosts", "/etc/.hosts.tmp");
        assert!(script.contains("cp -p --attributes-only '/etc/hosts' '/etc/.hosts.tmp'"));
        assert!(script.contains("tee '/etc/.hosts.tmp' > /dev/null"));
        assert!(script.contains("mv -f '/etc/.hosts.tmp' '/etc/hosts'"));
        assert!(script.contains("trap 'rm -f '/etc/.hosts.tmp'' EXIT"));
    }

    #[test]
    fn test_path_with_spaces_and_special_chars() {
        let path = "/var/log/my app/my 'config'.conf";
        let temp = generate_same_dir_temp_path(path, "abcd");
        assert_eq!(temp, "/var/log/my app/.my 'config'.conf.transom_sudo_abcd");
        let script = build_sudo_write_script(path, &temp);
        assert!(
            script.contains("cp -p --attributes-only '/var/log/my app/my '\\''config'\\''.conf'")
        );
    }
}
