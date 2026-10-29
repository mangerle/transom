//! SFTP 远程归档打包（tar / zip）与远程解压执行处理。
//!
//! 设计原理（Why）：
//! - 实现初衷：SFTP 协议本身不具备归档压缩与解压指令，通过复用底层 SSH 命令执行通道（`exec_command_capture`），
//!   在服务器端原生调用 `tar` 或 `unzip` 工具链，避免将大量小文件逐个经网络传输消耗带宽与 IO；
//! - 核心优势：
//!   1. 路径采用 POSIX 单引号全闭合安全转义（`posix_shell_quote`），杜绝远程命令注入与特殊字符截断漏洞；
//!   2. 自动根据目标文件扩展名识别最佳命令（`.tar.gz` / `.tgz` / `.tar` / `.zip`）；
//!   3. 解压前自动预创建目标目录（`mkdir -p`），保证解压至新子文件夹时的稳健性；
//! - 代价与局限：依赖远端宿主环境预装了 `tar` 或 `unzip` 命令，极精简嵌入式环境可能缺失对应命令工具。

use super::SftpExecutionContext;
use crate::controller::sftp_finish::finish_sftp_none;
use transom_core::{AppError, ErrorCode};

/// 对命令行参数进行标准的 POSIX 单引号转义闭合，防止 Shell 注入漏洞。
pub(crate) fn posix_shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// 执行远程归档打包压缩。
pub(super) async fn handle_archive(
    ctx: &SftpExecutionContext,
    source_paths: Vec<String>,
    dest_archive_path: String,
) {
    let result = execute_archive(ctx, source_paths, dest_archive_path).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

/// 构建归档打包 Shell 命令。
pub(crate) fn build_archive_command(source_paths: &[String], dest_archive_path: &str) -> String {
    let quoted_dest = posix_shell_quote(dest_archive_path);
    let quoted_sources = source_paths
        .iter()
        .map(|path| posix_shell_quote(path))
        .collect::<Vec<_>>()
        .join(" ");

    let lower_dest = dest_archive_path.to_ascii_lowercase();
    if lower_dest.ends_with(".zip") {
        format!("zip -q -r {quoted_dest} {quoted_sources}")
    } else if lower_dest.ends_with(".tar") {
        format!("tar -cf {quoted_dest} {quoted_sources}")
    } else {
        // 默认按 .tar.gz / .tgz 处理
        format!("tar -czf {quoted_dest} {quoted_sources}")
    }
}

/// 构建解压 Shell 命令。
pub(crate) fn build_extract_command(archive_path: &str, dest_dir: &str) -> String {
    let quoted_archive = posix_shell_quote(archive_path);
    let quoted_dest_dir = posix_shell_quote(dest_dir);

    let lower_archive = archive_path.to_ascii_lowercase();
    if lower_archive.ends_with(".zip") {
        format!("mkdir -p {quoted_dest_dir} && unzip -q -o {quoted_archive} -d {quoted_dest_dir}")
    } else if lower_archive.ends_with(".tar") {
        format!("mkdir -p {quoted_dest_dir} && tar -xf {quoted_archive} -C {quoted_dest_dir}")
    } else {
        // 默认按 .tar.gz / .tgz 处理
        format!("mkdir -p {quoted_dest_dir} && tar -xzf {quoted_archive} -C {quoted_dest_dir}")
    }
}

async fn execute_archive(
    ctx: &SftpExecutionContext,
    source_paths: Vec<String>,
    dest_archive_path: String,
) -> Result<(), AppError> {
    let Some(ref remote) = ctx.remote else {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            "当前会话缺少底层 SSH 执行通道，无法进行远程压缩",
        ));
    };

    if source_paths.is_empty() {
        return Ok(());
    }

    let command = build_archive_command(&source_paths, &dest_archive_path);
    log::info!("正在执行远程归档命令: {command}");
    let output = remote.exec_command_capture(&command).await?;
    if output.exit_status == Some(0) {
        log::info!("远程归档完成: {dest_archive_path}");
        Ok(())
    } else {
        let err_msg = if !output.stderr.trim().is_empty() {
            output.stderr.trim().to_string()
        } else {
            format!("归档进程退出异常 (退出码: {:?})", output.exit_status)
        };
        log::warn!("远程归档失败: {err_msg}");
        Err(AppError::with_detail(ErrorCode::SftpFailed, err_msg))
    }
}

/// 执行远程解压缩。
pub(super) async fn handle_extract(
    ctx: &SftpExecutionContext,
    archive_path: String,
    dest_dir: String,
) {
    let result = execute_extract(ctx, archive_path, dest_dir).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn execute_extract(
    ctx: &SftpExecutionContext,
    archive_path: String,
    dest_dir: String,
) -> Result<(), AppError> {
    let Some(ref remote) = ctx.remote else {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            "当前会话缺少底层 SSH 执行通道，无法进行远程解压",
        ));
    };

    let command = build_extract_command(&archive_path, &dest_dir);
    log::info!("正在执行远程解压命令: {command}");
    let output = remote.exec_command_capture(&command).await?;
    if output.exit_status == Some(0) {
        log::info!("远程解压完成: {archive_path} -> {dest_dir}");
        Ok(())
    } else {
        let err_msg = if !output.stderr.trim().is_empty() {
            output.stderr.trim().to_string()
        } else {
            format!("解压进程退出异常 (退出码: {:?})", output.exit_status)
        };
        log::warn!("远程解压失败: {err_msg}");
        Err(AppError::with_detail(ErrorCode::SftpFailed, err_msg))
    }
}

#[cfg(test)]
mod tests {
    use super::{build_archive_command, build_extract_command, posix_shell_quote};

    #[test]
    fn test_posix_shell_quote() {
        assert_eq!(posix_shell_quote("hello"), "'hello'");
        assert_eq!(posix_shell_quote("hello world"), "'hello world'");
        assert_eq!(posix_shell_quote("it's cool"), r"'it'\''s cool'");
        assert_eq!(posix_shell_quote(""), "''");
    }

    #[test]
    fn test_build_archive_command() {
        let sources = vec![
            "/var/log/app.log".to_string(),
            "/var/log/sys.log".to_string(),
        ];
        let cmd_tar_gz = build_archive_command(&sources, "/tmp/logs.tar.gz");
        assert_eq!(
            cmd_tar_gz,
            "tar -czf '/tmp/logs.tar.gz' '/var/log/app.log' '/var/log/sys.log'"
        );

        let cmd_zip = build_archive_command(&sources, "/tmp/logs.zip");
        assert_eq!(
            cmd_zip,
            "zip -q -r '/tmp/logs.zip' '/var/log/app.log' '/var/log/sys.log'"
        );

        let cmd_tar = build_archive_command(&sources, "/tmp/logs.tar");
        assert_eq!(
            cmd_tar,
            "tar -cf '/tmp/logs.tar' '/var/log/app.log' '/var/log/sys.log'"
        );
    }

    #[test]
    fn test_build_extract_command() {
        let cmd_tar_gz = build_extract_command("/tmp/app.tar.gz", "/var/www");
        assert_eq!(
            cmd_tar_gz,
            "mkdir -p '/var/www' && tar -xzf '/tmp/app.tar.gz' -C '/var/www'"
        );

        let cmd_zip = build_extract_command("/tmp/app.zip", "/var/www");
        assert_eq!(
            cmd_zip,
            "mkdir -p '/var/www' && unzip -q -o '/tmp/app.zip' -d '/var/www'"
        );

        let cmd_tar = build_extract_command("/tmp/app.tar", "/var/www");
        assert_eq!(
            cmd_tar,
            "mkdir -p '/var/www' && tar -xf '/tmp/app.tar' -C '/var/www'"
        );
    }
}
