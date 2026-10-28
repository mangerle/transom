//! 凭据模板（Identity）：可被多个会话共享的用户名与认证方式。
//!
//! 设计原理（Why）：
//! - 实现初衷：几十台机器共用同一个运维账号时，口令与私钥口令在会话里各存一份，
//!   改密就得逐台重填；把"谁登录"从"登录哪台"里拆出来，改一处即全量生效；
//! - 核心优势：模板只承载身份（用户名 + 认证方式 + 钥匙串槽位），主机、端口、
//!   WoL、转发等仍属会话，两者正交，引用关系是可选的——不引用的会话行为与
//!   改动前完全一致，老会话文件无需迁移；
//! - 代价与局限：口令本身仍只存在于系统钥匙串（模板里不落明文），因此跨机器
//!   带走身份必须走备份导出；模板被会话引用时不允许直接删除，避免留下
//!   "连不上又不知道为什么"的悬空引用。

use crate::ids::IdentityId;
use crate::model::session::AuthMethod;
use crate::secrets::SecretKind;
use serde::{Deserialize, Serialize};

/// 一份可复用的登录身份。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Identity {
    pub id: IdentityId,
    /// 展示名，同时也是搜索与列表主键；允许重名（不同团队可能同名账号）。
    pub name: String,
    /// 登录用户名；引用它的会话在连接时以此为准。
    pub username: String,
    /// 认证方式；口令明文交给钥匙串，这里只记"用哪种方式、存不存"。
    pub auth: AuthMethod,
}

impl Identity {
    pub fn new(name: impl Into<String>, username: impl Into<String>, auth: AuthMethod) -> Self {
        Self {
            id: IdentityId::new(),
            name: name.into(),
            username: username.into(),
            auth,
        }
    }

    /// 该模板可能在钥匙串里留下的口令种类。
    ///
    /// 与 `Session::secret_kinds` 同一语义：备份导出与删除模板时都要按它清理
    /// 槽位，否则明文备份会漏掉模板口令，删模板也会留下孤儿条目。
    pub fn secret_kinds(&self) -> &'static [SecretKind] {
        match &self.auth {
            AuthMethod::Password { .. } => &[SecretKind::Password],
            AuthMethod::PrivateKey { .. } => &[SecretKind::KeyPassphrase],
            AuthMethod::Agent => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn secret_kinds_follow_auth_method() {
        let password = Identity::new("ops", "ops", AuthMethod::Password { save: true });
        assert_eq!(password.secret_kinds(), &[SecretKind::Password]);

        let key = Identity::new(
            "ci",
            "ci",
            AuthMethod::PrivateKey {
                path: PathBuf::from("/srv/.ssh/id_ed25519"),
                cert_path: None,
                save_passphrase: true,
            },
        );
        assert_eq!(key.secret_kinds(), &[SecretKind::KeyPassphrase]);

        let agent = Identity::new("agent", "deploy", AuthMethod::Agent);
        assert!(agent.secret_kinds().is_empty());
    }

    #[test]
    fn round_trips_through_json() {
        let identity = Identity::new("ops", "ops", AuthMethod::Password { save: false });
        let json = serde_json::to_string(&identity).unwrap();
        let decoded: Identity = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, identity);
    }
}
