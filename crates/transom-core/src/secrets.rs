use crate::ids::SessionId;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    Password,
    KeyPassphrase,
    /// 云同步端点凭据：WebDAV 应用密码或 S3 Secret Access Key。
    ///
    /// 与会话口令分属不同钥匙串槽位：端点凭据不属于任何会话，也不进入
    /// 会话备份的口令快照，只服务于云端同步的上传下载认证。
    CloudSyncCredential,
    /// 云同步口令：加密备份内容的 Argon2id 派生源，跨设备必须一致。
    CloudSyncPassphrase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKey {
    pub session_id: SessionId,
    pub kind: SecretKind,
}

impl SecretKey {
    pub fn new(session_id: SessionId, kind: SecretKind) -> Self {
        Self { session_id, kind }
    }

    /// 云同步端点凭据的固定槽位。
    ///
    /// 端点凭据没有归属会话，用全零 UUID 占位即可；钥匙串账号串由
    /// `account()` 按种类特判，保证唯一且跨设备稳定。
    pub fn cloud_sync_credential() -> Self {
        Self::new(Self::nil_session(), SecretKind::CloudSyncCredential)
    }

    /// 云同步口令的固定槽位。
    pub fn cloud_sync_passphrase() -> Self {
        Self::new(Self::nil_session(), SecretKind::CloudSyncPassphrase)
    }

    fn nil_session() -> SessionId {
        SessionId::from(uuid::Uuid::nil())
    }

    pub fn account(&self) -> String {
        let kind = match self.kind {
            SecretKind::Password => "password",
            SecretKind::KeyPassphrase => "key_passphrase",
            // 云同步两类槽位不携带会话语境，账号串用固定名而非 UUID 前缀，
            // 避免在钥匙串里暴露一串无意义的全零 UUID。
            SecretKind::CloudSyncCredential => return "cloud-sync/credential".to_string(),
            SecretKind::CloudSyncPassphrase => return "cloud-sync/passphrase".to_string(),
        };
        format!("{}/{kind}", self.session_id)
    }
}

#[derive(Default)]
pub struct ConnectionSecrets {
    pub password: Option<Zeroizing<String>>,
    pub key_passphrase: Option<Zeroizing<String>>,
}
