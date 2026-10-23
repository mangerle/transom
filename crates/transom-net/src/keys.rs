//! 通过已建立的 SSH 会话把公钥追加到远端 `authorized_keys`。

use crate::client::SshRemote;
use russh::keys::PublicKey;
use transom_core::{AppError, ErrorCode};

/// 将单行 OpenSSH 公钥部署到远端用户的 `~/.ssh/authorized_keys`。
///
/// 设计原理（Why）：
/// - 复用当前已验证的 SSH 连接执行一次幂等 shell 脚本，无需再次询问密码；
/// - 先 `grep -qxF` 精确匹配再去重追加，避免重复部署产生冗余条目；
/// - 所有路径变量均加引号，公钥本身按 POSIX 单引号规则转义，
///   防止注释中的特殊字符造成命令注入；
/// - 目录与文件权限收紧为 0700 / 0600，满足 OpenSSH 对 `authorized_keys` 的要求。
///
/// # Errors
/// 公钥格式非法时返回 `Configuration`；通道执行失败或远端退出码非零时
/// 返回 `ChannelFailed`，并附带远端错误输出。
pub async fn deploy_public_key(remote: &SshRemote, public_key: &str) -> Result<(), AppError> {
    let script = deployment_script(public_key)?;
    let output = remote.exec_command_capture(&script).await?;
    if output.exit_status == Some(0) {
        return Ok(());
    }
    Err(AppError::with_detail(
        ErrorCode::ChannelFailed,
        format!(
            "部署公钥失败，远端退出码: {}，错误输出: {}",
            output
                .exit_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "未返回".to_string()),
            if output.stderr.trim().is_empty() {
                "(无)"
            } else {
                output.stderr.trim()
            }
        ),
    ))
}

fn deployment_script(public_key: &str) -> Result<String, AppError> {
    let key = public_key.trim();
    if key.is_empty() || key.contains('\n') || key.contains('\r') || key.contains('\0') {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "公钥必须是非空的单行 OpenSSH 文本",
        ));
    }
    PublicKey::from_openssh(key).map_err(|error| {
        AppError::with_detail(ErrorCode::Configuration, format!("公钥格式无效: {error}"))
    })?;
    let quoted = shell_single_quote(key);
    Ok(format!(
        "umask 077; mkdir -p \"$HOME/.ssh\" && chmod 700 \"$HOME/.ssh\" \
         && touch \"$HOME/.ssh/authorized_keys\" && chmod 600 \"$HOME/.ssh/authorized_keys\" \
         && {{ grep -qxF -- {quoted} \"$HOME/.ssh/authorized_keys\" \
         || printf '%s\\n' {quoted} >> \"$HOME/.ssh/authorized_keys\"; }}"
    ))
}

/// 按 POSIX shell 规则把任意文本包裹为单引号字面量。
fn shell_single_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey, key::safe_rng};

    #[test]
    fn quotes_single_quotes_for_posix_shell() {
        assert_eq!(shell_single_quote("plain"), "'plain'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_single_quote("$HOME `date`"), "'$HOME `date`'");
    }

    #[test]
    fn deployment_script_is_idempotent_and_quotes_key() {
        let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).expect("生成测试密钥");
        let openssh = key.public_key().to_openssh().expect("编码测试公钥");
        let script = deployment_script(&openssh).expect("构建部署脚本");
        assert!(script.contains("grep -qxF --"));
        assert!(script.contains(&shell_single_quote(&openssh)));
        assert!(script.contains("chmod 600 \"$HOME/.ssh/authorized_keys\""));
        assert!(script.contains("umask 077"));
    }

    #[test]
    fn rejects_invalid_public_keys() {
        assert!(deployment_script("").is_err());
        assert!(deployment_script("ssh-ed25519 not-a-key").is_err());
        assert!(deployment_script("ssh-ed25519 AAAA\nssh-ed25519 BBBB").is_err());
    }
}
