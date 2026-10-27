//! 客户端握手、认证与输出限制单元测试。

use super::auth::{request_secret, sha1_signature_required};
use super::handler::ClientHandler;
use super::jump::ssh_preferred;
use super::session::{
    COMMAND_EXCERPT_BYTES, MAX_EXEC_OUTPUT_BYTES, command_excerpt, exec_output_exceeds,
};
use crate::auth::AuthPrompter;
use parking_lot::Mutex;
use russh::client::Handler;
use russh::keys::key::safe_rng;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKeyOrCertificate};
use std::collections::HashMap;
use std::sync::Arc;
use transom_core::{
    AppError, ErrorCode, HostKeyInfo, HostKeyStatus, KnownHostsStore, PromptInfo, PromptKind,
};

/// 测试替身：按预设答案回应认证索取，`Err` 分支模拟用户取消；
/// `host_key` 决定 TOFU 指纹确认的答复，模拟用户信任或拒绝。
struct ScriptedPrompter {
    answers: Result<Vec<String>, ()>,
    host_key: bool,
}

#[async_trait::async_trait]
impl AuthPrompter for ScriptedPrompter {
    async fn confirm_host_key(
        &self,
        _host: &str,
        _port: u16,
        _key_type: &str,
        _fingerprint: &str,
    ) -> Result<bool, AppError> {
        Ok(self.host_key)
    }

    async fn keyboard_interactive(
        &self,
        _name: &str,
        _instructions: &str,
        prompts: &[PromptInfo],
    ) -> Result<Vec<String>, AppError> {
        self.answers.clone().map_err(|_| {
            AppError::with_detail(
                ErrorCode::Cancelled,
                format!("模拟用户取消认证，待回答提示数: {}", prompts.len()),
            )
        })
    }
}

struct MemoryKnownHosts {
    inner: Mutex<Vec<(String, u16, String)>>,
}

impl KnownHostsStore for MemoryKnownHosts {
    fn check(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<HostKeyStatus, AppError> {
        let inner = self.inner.lock();
        match inner
            .iter()
            .find(|(saved_host, saved_port, _)| saved_host == host && *saved_port == port)
        {
            Some((_, _, fingerprint)) if fingerprint == &info.fingerprint => {
                Ok(HostKeyStatus::Match)
            }
            Some((_, _, fingerprint)) => Ok(HostKeyStatus::Mismatch {
                previous_fingerprint: fingerprint.clone(),
            }),
            None => Ok(HostKeyStatus::Unknown),
        }
    }

    fn record(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<(), AppError> {
        self.inner
            .lock()
            .push((host.to_owned(), port, info.fingerprint.clone()));
        Ok(())
    }

    fn remove(&self, _host: &str, _port: u16) -> Result<(), AppError> {
        Ok(())
    }

    fn list(&self) -> Result<Vec<transom_core::HostKeyRecord>, AppError> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn handler_records_unknown_and_rejects_changed_keys() {
    let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
    let public = key.public_key();
    let server_key = PublicKeyOrCertificate::from(public.clone());

    let known_hosts = Arc::new(MemoryKnownHosts {
        inner: Mutex::new(Vec::new()),
    });
    let mut handler = ClientHandler {
        host: "example.com".into(),
        port: 22,
        known_hosts: known_hosts.clone(),
        remote_forwards: Arc::new(std::sync::Mutex::new(HashMap::new())),
        prompter: Arc::new(ScriptedPrompter {
            answers: Ok(Vec::new()),
            host_key: true,
        }),
    };

    assert!(handler.check_server_key(&server_key).await.unwrap());
    assert!(handler.check_server_key(&server_key).await.unwrap());
    assert_eq!(known_hosts.inner.lock().len(), 1);

    let other = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
    let other_key = PublicKeyOrCertificate::from(other.public_key().clone());
    assert!(handler.check_server_key(&other_key).await.is_err());
}

/// TOFU 拒绝路径：用户不确认指纹时连接必须失败，且不得写入 known_hosts。
#[tokio::test]
async fn handler_rejected_host_key_is_not_recorded() {
    let key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
    let server_key = PublicKeyOrCertificate::from(key.public_key().clone());
    let known_hosts = Arc::new(MemoryKnownHosts {
        inner: Mutex::new(Vec::new()),
    });
    let mut handler = ClientHandler {
        host: "example.com".into(),
        port: 22,
        known_hosts: known_hosts.clone(),
        remote_forwards: Arc::new(std::sync::Mutex::new(HashMap::new())),
        prompter: Arc::new(ScriptedPrompter {
            answers: Ok(Vec::new()),
            host_key: false,
        }),
    };
    assert!(handler.check_server_key(&server_key).await.is_err());
    assert!(
        known_hosts.inner.lock().is_empty(),
        "拒绝指纹时绝不能记录信任"
    );
}

#[tokio::test]
async fn request_secret_returns_the_prompted_answer() {
    let prompter = ScriptedPrompter {
        answers: Ok(vec!["s3cret".to_string()]),
        host_key: true,
    };
    let secret = request_secret(&prompter, PromptKind::Password)
        .await
        .expect("应当取回用户输入的凭据");
    assert_eq!(&*secret, "s3cret");
}

#[tokio::test]
async fn request_secret_fails_on_empty_or_cancelled_input() {
    let blank = ScriptedPrompter {
        answers: Ok(vec![String::new()]),
        host_key: true,
    };
    assert!(
        request_secret(&blank, PromptKind::Passphrase)
            .await
            .is_err()
    );

    let cancelled = ScriptedPrompter {
        answers: Err(()),
        host_key: true,
    };
    assert!(
        request_secret(&cancelled, PromptKind::Password)
            .await
            .is_err()
    );
}

#[test]
fn ssh_preferred_drops_sha1_rsa_host_keys_but_keeps_modern_algorithms() {
    let preferred = ssh_preferred();
    assert!(
        !preferred
            .key
            .iter()
            .any(|algorithm| matches!(algorithm, Algorithm::Rsa { hash: None })),
        "不能接受 SHA1 的 ssh-rsa 主机签名"
    );
    assert!(
        preferred.key.contains(&Algorithm::Ed25519),
        "Ed25519 主机密钥必须保留"
    );
    assert!(
        preferred.key.contains(&Algorithm::Rsa {
            hash: Some(HashAlg::Sha256)
        }),
        "RSA 主机密钥仍应可走 SHA2 签名"
    );
    assert!(
        !preferred.cipher.is_empty(),
        "过滤主机密钥不应连带清空加密套件列表"
    );
}

/// 输出上限的边界：stdout 与 stderr 合计恰好等于上限放行，超一字节即拒绝。
#[test]
fn exec_output_limit_counts_stdout_and_stderr_together() {
    let half = MAX_EXEC_OUTPUT_BYTES / 2;
    assert!(
        !exec_output_exceeds(&vec![0u8; MAX_EXEC_OUTPUT_BYTES], &[], 0),
        "恰好等于上限不应判定超限"
    );
    assert!(
        exec_output_exceeds(&vec![0u8; MAX_EXEC_OUTPUT_BYTES], &[], 1),
        "超出上限一字节必须拒绝"
    );
    assert!(
        !exec_output_exceeds(&vec![0u8; half], &vec![0u8; half], 0),
        "stdout 与 stderr 各半、合计等于上限不应判定超限"
    );
    assert!(
        exec_output_exceeds(&vec![0u8; half], &vec![0u8; half], 1),
        "合计超出上限一字节必须拒绝"
    );
}

/// 命令摘要按字符边界截断：短命令原样返回，长命令带省略标记且不切开多字节字符。
#[test]
fn command_excerpt_truncates_on_char_boundary() {
    assert_eq!(command_excerpt("kill -9 1234"), "kill -9 1234");
    let long = "公钥部署命令".repeat(40);
    let excerpt = command_excerpt(&long);
    assert!(
        excerpt.len() <= COMMAND_EXCERPT_BYTES + 3,
        "截断后长度不得超过上限加省略标记"
    );
    assert!(excerpt.ends_with("..."), "截断必须带省略标记");
    assert!(excerpt.starts_with("公钥部署命令"), "截断不得破坏首段文本");
}

/// SHA1 判定的组合边界：只有"RSA 私钥 + 无 SHA2 哈希"才触发准入检查。
#[test]
fn sha1_signature_required_only_for_rsa_without_sha2() {
    let rsa_sha1 = Algorithm::Rsa { hash: None };
    assert!(sha1_signature_required(&rsa_sha1, None));
    assert!(!sha1_signature_required(&rsa_sha1, Some(HashAlg::Sha256)));
    assert!(!sha1_signature_required(&Algorithm::Ed25519, None));
    assert!(!sha1_signature_required(
        &Algorithm::Ed25519,
        Some(HashAlg::Sha512)
    ));
}
