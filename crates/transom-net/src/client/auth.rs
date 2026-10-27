//! SSH 客户端认证管线：口令重试、私钥与证书加载、Agent 与交互式键盘认证。

use super::handler::ClientHandler;
use crate::auth::AuthPrompter;
use crate::error::NetError;
use russh::client::Handle;
use russh::keys::agent::client::AgentClient;
use russh::keys::{Algorithm, HashAlg, PrivateKeyWithHashAlg, load_secret_key};
use std::sync::Arc;
use transom_core::{AppError, ConnectionSecrets, ErrorCode, PromptInfo, PromptKind, SshConnection};
use zeroize::Zeroizing;

/// 密码认证的尝试上限，对齐 OpenSSH 给用户的三次机会。
pub(crate) const MAX_PASSWORD_ATTEMPTS: usize = 3;

pub(crate) async fn authenticate(
    session: &mut Handle<ClientHandler>,
    connection: &SshConnection,
    mut credentials: ConnectionSecrets,
    prompter: Arc<dyn AuthPrompter>,
    allow_sha1_publickey_auth: bool,
) -> Result<(), AppError> {
    let user = connection.username.clone();
    let authenticated = match &connection.auth {
        transom_core::AuthMethod::Password { .. } => {
            password_flow(
                session,
                &user,
                prompter.as_ref(),
                credentials.password.take(),
            )
            .await?
        }
        transom_core::AuthMethod::PrivateKey { .. } => {
            authenticate_private_key(
                session,
                connection,
                &mut credentials,
                prompter.as_ref(),
                allow_sha1_publickey_auth,
            )
            .await?
        }
        transom_core::AuthMethod::Agent => authenticate_agent(session, &user).await?,
        _ => {
            return Err(NetError::Auth {
                reason: "不支持的认证方式".to_string(),
            }
            .into());
        }
    };

    if authenticated {
        return Ok(());
    }

    if connection.options.allow_keyboard_interactive
        && authenticate_keyboard_interactive(session, &user, prompter).await?
    {
        return Ok(());
    }

    Err(NetError::Auth {
        reason: "服务器拒绝了全部认证方式".to_string(),
    }
    .into())
}

/// 密码认证流程：先用手上的凭据试，被服务器拒绝后重新索取，最多三次。
async fn password_flow(
    session: &mut Handle<ClientHandler>,
    user: &str,
    prompter: &dyn AuthPrompter,
    stored: Option<Zeroizing<String>>,
) -> Result<bool, AppError> {
    let mut candidate = stored;
    for attempt in 0..MAX_PASSWORD_ATTEMPTS {
        let password = match candidate.take() {
            Some(password) => password,
            None => request_secret(prompter, PromptKind::Password).await?,
        };
        let accepted = session
            .authenticate_password(user.to_string(), password.to_string())
            .await
            .map_err(auth_error)?
            .success();
        if accepted {
            return Ok(true);
        }
        if attempt + 1 < MAX_PASSWORD_ATTEMPTS {
            log::warn!(
                "密码被服务器拒绝，将在本次认证中重新询问，用户: {user}，本次第 {} 次，上限 {} 次",
                attempt + 1,
                MAX_PASSWORD_ATTEMPTS
            );
        }
    }
    Ok(false)
}

/// 用私钥完成认证，可选附带企业 CA 签发的 OpenSSH 用户证书。
async fn authenticate_private_key(
    session: &mut Handle<ClientHandler>,
    connection: &SshConnection,
    credentials: &mut ConnectionSecrets,
    prompter: &dyn AuthPrompter,
    allow_sha1_publickey_auth: bool,
) -> Result<bool, AppError> {
    let transom_core::AuthMethod::PrivateKey {
        path, cert_path, ..
    } = &connection.auth
    else {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "内部错误：认证方式不是私钥却进入了私钥分支",
        ));
    };
    let user = connection.username.clone();
    let key = match load_secret_key(path, credential_ref(&credentials.key_passphrase)) {
        Ok(key) => key,
        Err(first_error) => match &credentials.key_passphrase {
            Some(_) => return Err(load_key_error(path, &first_error)),
            None => {
                let passphrase = request_secret(prompter, PromptKind::Passphrase).await?;
                load_secret_key(path, Some(passphrase.as_str()))
                    .map_err(|error| load_key_error(path, &error))?
            }
        },
    };
    let hash_alg = session
        .best_supported_rsa_hash()
        .await
        .map_err(auth_error)?
        .flatten();
    ensure_rsa_signature_allowed(&key, hash_alg, &connection.host, allow_sha1_publickey_auth)?;
    let key = Arc::new(key);
    let Some(cert_path) = cert_path else {
        return pubkey_auth(session, &user, key, hash_alg).await;
    };
    let certificate = russh::keys::load_openssh_certificate(cert_path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::AuthFailed,
            format!(
                "加载 SSH 用户证书失败，路径: {}，原因: {error}",
                cert_path.display()
            ),
        )
    })?;
    let via_certificate = session
        .authenticate_openssh_cert(user.clone(), Arc::clone(&key), certificate)
        .await
        .map_err(auth_error)?;
    if via_certificate.success() {
        return Ok(true);
    }
    log::warn!(
        "证书认证被服务器拒绝，回退到公钥认证，主机: {}，证书: {}",
        connection.host,
        cert_path.display()
    );
    pubkey_auth(session, &user, key, hash_alg).await
}

async fn pubkey_auth(
    session: &mut Handle<ClientHandler>,
    user: &str,
    key: Arc<russh::keys::PrivateKey>,
    hash_alg: Option<HashAlg>,
) -> Result<bool, AppError> {
    session
        .authenticate_publickey(user.to_owned(), PrivateKeyWithHashAlg::new(key, hash_alg))
        .await
        .map_err(auth_error)
        .map(|result| result.success())
}

pub(crate) fn sha1_signature_required(
    key_algorithm: &Algorithm,
    hash_alg: Option<HashAlg>,
) -> bool {
    hash_alg.is_none() && matches!(key_algorithm, Algorithm::Rsa { .. })
}

fn ensure_rsa_signature_allowed(
    key: &russh::keys::PrivateKey,
    hash_alg: Option<HashAlg>,
    host: &str,
    allow_sha1: bool,
) -> Result<(), AppError> {
    if !sha1_signature_required(&key.algorithm(), hash_alg) {
        return Ok(());
    }
    if allow_sha1 {
        log::warn!(
            "服务器 {host} 只支持 SHA1 用户公钥签名（ssh-rsa），已按兼容模式继续；如需收紧，请在设置文件中关闭 allow_sha1_publickey_auth"
        );
        return Ok(());
    }
    Err(AppError::with_detail(
        ErrorCode::AuthFailed,
        format!(
            "服务器 {host} 只支持 SHA1（ssh-rsa）用户公钥签名，当前设置已禁止；如确需连接该设备，请在设置文件中开启 allow_sha1_publickey_auth"
        ),
    ))
}

fn credential_ref(value: &Option<Zeroizing<String>>) -> Option<&str> {
    value.as_ref().map(|secret| secret.as_str())
}

fn load_key_error(path: &std::path::Path, error: &russh::keys::Error) -> AppError {
    NetError::Auth {
        reason: format!("加载私钥失败 {}: {error}", path.display()),
    }
    .into()
}

pub(crate) async fn request_secret(
    prompter: &dyn AuthPrompter,
    kind: PromptKind,
) -> Result<Zeroizing<String>, AppError> {
    let answers = prompter
        .keyboard_interactive("", "", &[PromptInfo::secret(kind)])
        .await?;
    let answer = answers
        .first()
        .filter(|answer| !answer.is_empty())
        .ok_or_else(|| NetError::Cancelled {
            reason: "凭据输入为空".to_string(),
        })?;
    Ok(Zeroizing::new(answer.clone()))
}

fn auth_error(error: russh::Error) -> AppError {
    AppError::with_detail(ErrorCode::AuthFailed, error.to_string())
}

fn agent_error(error: impl std::fmt::Display) -> NetError {
    NetError::Auth {
        reason: format!("SSH 代理（ssh-agent）认证失败: {error}"),
    }
}

async fn authenticate_agent(
    session: &mut Handle<ClientHandler>,
    user: &str,
) -> Result<bool, AppError> {
    #[cfg(unix)]
    {
        let mut agent = AgentClient::connect_env().await.map_err(agent_error)?;
        let identities = agent.request_identities().await.map_err(agent_error)?;
        for identity in identities {
            let public_key = identity.public_key().into_owned();
            let result = session
                .authenticate_publickey_with(user.to_owned(), public_key, None, &mut agent)
                .await
                .map_err(agent_error)?;
            if result.success() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    #[cfg(not(unix))]
    {
        let _ = (session, user);
        Err(NetError::Auth {
            reason: "当前平台暂不支持 ssh-agent 认证".to_string(),
        }
        .into())
    }
}

async fn authenticate_keyboard_interactive(
    session: &mut Handle<ClientHandler>,
    user: &str,
    prompter: Arc<dyn AuthPrompter>,
) -> Result<bool, AppError> {
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut response = session
        .authenticate_keyboard_interactive_start(user.to_owned(), None)
        .await
        .map_err(auth_error)?;

    for _ in 0..8 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let prompt_info: Vec<PromptInfo> = prompts
                    .iter()
                    .map(|prompt| PromptInfo {
                        prompt: prompt.prompt.clone(),
                        echo: prompt.echo,
                        kind: PromptKind::Text,
                    })
                    .collect();
                let answers = prompter
                    .keyboard_interactive(&name, &instructions, &prompt_info)
                    .await?;
                if answers.len() != prompt_info.len() {
                    return Err(AppError::with_detail(
                        ErrorCode::KeyboardInteractiveFailed,
                        "prompt response count mismatch",
                    ));
                }
                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(auth_error)?;
            }
        }
    }

    Err(AppError::with_detail(
        ErrorCode::KeyboardInteractiveFailed,
        "too many keyboard-interactive rounds",
    ))
}
