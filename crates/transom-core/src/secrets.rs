use crate::ids::{IdentityId, SessionId};
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
    /// 私有 Git 仓库的 HTTPS 访问令牌；SSH 远端不需要。
    GitSyncToken,
}

/// 口令归属：某个会话，或某个可被多个会话引用的凭据模板。
///
/// 为什么需要这一层：会话引用凭据模板时，口令必须只存一份、改一处生效；
/// 若继续按 SessionId 归档，同一密码会在几十个会话的钥匙串槽里各存一份，
/// 改密就退化成"逐台重填"。类型上区分归属，也杜绝把 IdentityId 当
/// SessionId 传进 `SecretKey::new` 的错用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretOwner {
    Session(SessionId),
    Identity(IdentityId),
}

impl From<SessionId> for SecretOwner {
    fn from(id: SessionId) -> Self {
        Self::Session(id)
    }
}

impl From<IdentityId> for SecretOwner {
    fn from(id: IdentityId) -> Self {
        Self::Identity(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKey {
    pub owner: SecretOwner,
    pub kind: SecretKind,
}

impl SecretKey {
    /// 构造口令槽位；调用方传 SessionId 或 IdentityId 均可，无需手写枚举包装。
    pub fn new(owner: impl Into<SecretOwner>, kind: SecretKind) -> Self {
        Self {
            owner: owner.into(),
            kind,
        }
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

    /// Git HTTPS 访问令牌的固定槽位。
    pub fn git_sync_token() -> Self {
        Self::new(Self::nil_session(), SecretKind::GitSyncToken)
    }

    fn nil_session() -> SessionId {
        SessionId::from(uuid::Uuid::nil())
    }

    /// 钥匙串账号串。
    ///
    /// 会话槽位必须保持 `<uuid>/<kind>` 的**既有形状**：已保存的口令靠这个
    /// 字符串定位，改名等于把用户的全部凭据弄丢。凭据模板另起 `identity/`
    /// 前缀命名空间，与任何会话 UUID 都不可能撞车。
    pub fn account(&self) -> String {
        let kind = match self.kind {
            SecretKind::Password => "password",
            SecretKind::KeyPassphrase => "key_passphrase",
            // 云同步两类槽位不携带会话语境，账号串用固定名而非 UUID 前缀，
            // 避免在钥匙串里暴露一串无意义的全零 UUID。
            SecretKind::CloudSyncCredential => return "cloud-sync/credential".to_string(),
            SecretKind::CloudSyncPassphrase => return "cloud-sync/passphrase".to_string(),
            SecretKind::GitSyncToken => return "git-sync/token".to_string(),
        };
        match self.owner {
            SecretOwner::Session(id) => format!("{id}/{kind}"),
            SecretOwner::Identity(id) => format!("identity/{id}/{kind}"),
        }
    }
}

#[derive(Default, Clone)]
pub struct ConnectionSecrets {
    pub password: Option<Zeroizing<String>>,
    pub key_passphrase: Option<Zeroizing<String>>,
}
