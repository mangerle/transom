//! 备份文档契约：一次导出的完整内容与配套选项。
//!
//! 设计说明（Why）：
//! - 备份内容除了会话目录快照，还可能携带系统钥匙串里的登录口令，因此单独
//!   定义文档类型而不直接复用 `SessionCatalog`：口令不属于会话本身，混进
//!   目录结构会让日常持久化的 `sessions.json` 变成明文密码库。
//! - 口令值用 `Zeroizing` 保活，文档被丢弃时立即归零，避免内存里留下长期
//!   驻留的明文副本；序列化按借用写出，不额外克隆口令。
//! - 加密与否由"有没有口令"表达，而不是额外的布尔开关：明文与加密是两种
//!   互斥的文件形态，用 `Option` 能让非法组合在类型层面无法出现。

use crate::catalog::{SessionCatalog, SessionFolder};
use crate::error::AppError;
use crate::identity::Identity;
use crate::ids::{IdentityId, SessionId};
use crate::secrets::{SecretKind, SecretOwner};
use crate::session::Session;
use crate::storage::SecretStore;
use serde::de::Deserializer;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

/// 按会话的认证方式收集钥匙串里保存的口令。
///
/// 本地备份导出与云端同步上传共用这一条收集路径：钥匙串整体不可用时直接
/// 失败——此时"含口令"的备份只会得到一份缺口的文件，用户很难察觉，必须
/// 让他显式取消勾选或修复钥匙串，而不是静默产出残缺备份。
///
/// # Errors
/// 任一会话的口令读取失败（钥匙串不可用）时返回 `AppError`，错误码为
/// [`ErrorCode::SecretStoreUnavailable`](crate::ErrorCode::SecretStoreUnavailable)。
pub fn collect_catalog_secrets(
    secrets: &dyn SecretStore,
    catalog: &SessionCatalog,
) -> Result<Vec<BackupSecret>, AppError> {
    let mut collected = Vec::new();
    for session in &catalog.sessions {
        for kind in session.secret_kinds() {
            let key = crate::secrets::SecretKey::new(session.id, *kind);
            match secrets.get(&key) {
                Ok(Some(value)) => collected.push(BackupSecret {
                    owner: SecretOwner::Session(session.id),
                    kind: *kind,
                    value,
                }),
                Ok(None) => {}
                Err(_error) => {
                    return Err(AppError::with_detail(
                        crate::ErrorCode::SecretStoreUnavailable,
                        format!(
                            "读取会话「{}」的口令失败：系统钥匙串不可用；请取消「包含已保存的口令」后重试",
                            session.name
                        ),
                    ));
                }
            }
        }
    }
    // 凭据模板的口令同样只有一份：不收集它们的话，换机导入后每个引用会话
    // 都会重新被问一遍密码，而"改一处生效"的前提就是这份口令能跟着目录走。
    for identity in &catalog.identities {
        for kind in identity.secret_kinds() {
            let key = crate::secrets::SecretKey::new(identity.id, *kind);
            match secrets.get(&key) {
                Ok(Some(value)) => collected.push(BackupSecret {
                    owner: SecretOwner::Identity(identity.id),
                    kind: *kind,
                    value,
                }),
                Ok(None) => {}
                Err(_error) => {
                    return Err(AppError::with_detail(
                        crate::ErrorCode::SecretStoreUnavailable,
                        format!(
                            "读取凭据模板「{}」的口令失败：系统钥匙串不可用；请取消「包含已保存的口令」后重试",
                            identity.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(collected)
}

/// 一次导出的完整内容。
///
/// 明文备份文件就是这份结构的 JSON/YAML 序列化结果：`folders` 与 `sessions`
/// 是会话目录快照，`secrets` 是用户显式勾选"包含口令"时才出现的口令数组。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BackupDocument {
    /// 文件夹节点；数组原始顺序即同层级显示顺序。
    #[serde(default)]
    pub folders: Vec<SessionFolder>,
    /// 会话清单。
    #[serde(default)]
    pub sessions: Vec<Session>,
    /// 凭据模板清单；老备份文件没有该字段时按空清单导入。
    #[serde(default)]
    pub identities: Vec<Identity>,
    /// 钥匙串口令快照；未勾选"包含口令"时整个字段不出现。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<BackupSecret>>,
}

impl BackupDocument {
    /// 由目录快照构造不含口令的备份文档。
    pub fn from_catalog(catalog: &SessionCatalog) -> Self {
        Self {
            folders: catalog.folders.clone(),
            sessions: catalog.sessions.clone(),
            identities: catalog.identities.clone(),
            secrets: None,
        }
    }

    /// 拆出目录快照与口令列表，交接所有权避免口令被额外克隆。
    pub fn into_parts(self) -> (SessionCatalog, Option<Vec<BackupSecret>>) {
        (
            SessionCatalog {
                folders: self.folders,
                sessions: self.sessions,
                identities: self.identities,
            },
            self.secrets,
        )
    }

    /// 修复跨机器数据可能出现的悬空父级与重复节点。
    ///
    /// 备份文件可能来自另一台机器，父子指针与 ID 都未必自洽，因此在进入
    /// 合并流程前先按本机规则归一。
    pub fn normalize(&mut self) {
        let mut catalog = SessionCatalog {
            folders: std::mem::take(&mut self.folders),
            sessions: std::mem::take(&mut self.sessions),
            identities: std::mem::take(&mut self.identities),
        };
        catalog.normalize();
        self.folders = catalog.folders;
        self.sessions = catalog.sessions;
    }
}

/// 备份中的一条钥匙串口令。
pub struct BackupSecret {
    /// 口令归属：会话本身或其引用的凭据模板。
    pub owner: SecretOwner,
    /// 口令种类：登录密码或私钥口令。
    pub kind: SecretKind,
    /// 口令明文；文档释放时归零。
    pub value: Zeroizing<String>,
}

impl fmt::Debug for BackupSecret {
    /// 口令值一律脱敏：备份文档会随错误信息与日志流转，明文不得外泄。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupSecret")
            .field("owner", &self.owner)
            .field("kind", &self.kind)
            .field("value", &"<已脱敏>")
            .finish()
    }
}

impl Serialize for BackupSecret {
    /// 会话口令写 `session_id`（与既有备份文件逐字节一致），模板口令写
    /// `identity_id`：两种归属各用一个字段名，老版本读到自己的字段就能用，
    /// 不必为区分归属再引入一层包装结构。
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("BackupSecret", 3)?;
        match self.owner {
            SecretOwner::Session(id) => state.serialize_field("session_id", &id)?,
            SecretOwner::Identity(id) => state.serialize_field("identity_id", &id)?,
        }
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("value", self.value.as_str())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BackupSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            session_id: Option<SessionId>,
            #[serde(default)]
            identity_id: Option<IdentityId>,
            kind: SecretKind,
            value: String,
        }

        let raw = Raw::deserialize(deserializer)?;
        // 两个字段都缺或同时出现都视为坏数据：宁可报错，也不猜归属，
        // 猜错会把口令写到别人的槽位上。
        let owner = match (raw.session_id, raw.identity_id) {
            (Some(id), None) => SecretOwner::Session(id),
            (None, Some(id)) => SecretOwner::Identity(id),
            _ => {
                return Err(serde::de::Error::custom(
                    "备份口令条目必须恰好带 session_id 或 identity_id 之一",
                ));
            }
        };
        Ok(Self {
            owner,
            kind: raw.kind,
            value: Zeroizing::new(raw.value),
        })
    }
}

/// 导出选项。
#[derive(Default)]
pub struct BackupExportOptions {
    /// 加密口令；`None` 表示导出明文文件。
    pub passphrase: Option<Zeroizing<String>>,
    /// 是否把系统钥匙串里的会话口令一并写入备份。
    pub include_secrets: bool,
}

impl fmt::Debug for BackupExportOptions {
    /// 口令只报告"是否加密"，绝不打印内容。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupExportOptions")
            .field("encrypted", &self.passphrase.is_some())
            .field("include_secrets", &self.include_secrets)
            .finish()
    }
}

/// 导入选项。
#[derive(Default)]
pub struct BackupImportOptions {
    /// 解密口令；`None` 表示按明文文件解析。
    pub passphrase: Option<Zeroizing<String>>,
}

impl fmt::Debug for BackupImportOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupImportOptions")
            .field("encrypted", &self.passphrase.is_some())
            .finish()
    }
}

/// 导出结果统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackupExportReport {
    /// 导出的会话数量。
    pub sessions: usize,
    /// 导出的文件夹数量。
    pub folders: usize,
    /// 写入备份的口令数量。
    pub secrets: usize,
    /// 是否以加密形式落盘。
    pub encrypted: bool,
}

/// 口令写回结果统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SecretRestoreReport {
    /// 新写入本机钥匙串的口令数量。
    pub restored: usize,
    /// 本机已存在、按"保留现有"规则跳过的口令数量。
    pub kept: usize,
    /// 写入失败的口令数量。
    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_password(name: &str) -> Session {
        Session::new(
            name,
            crate::Connection::Ssh(crate::SshConnection {
                host: "example.com".into(),
                port: 22,
                username: "deploy".into(),
                auth: crate::AuthMethod::Password { save: true },
                options: Default::default(),
                proxy_jump: None,
            }),
        )
    }

    /// 明文备份文件里不出现口令字段，除非用户显式带上。
    #[test]
    fn document_without_secrets_has_no_secret_field() {
        let mut catalog = SessionCatalog::default();
        catalog.sessions.push(session_with_password("web-01"));
        let document = BackupDocument::from_catalog(&catalog);
        let json = serde_json::to_string(&document).unwrap();
        assert!(!json.contains("secrets"));
        assert!(json.contains("web-01"));
    }

    /// 口令要能被序列化回去，且 Debug 输出不得泄露明文。
    #[test]
    fn secrets_round_trip_and_stay_redacted_in_debug() {
        let session = session_with_password("web-01");
        let document = BackupDocument {
            folders: Vec::new(),
            sessions: vec![session.clone()],
            identities: Vec::new(),
            secrets: Some(vec![BackupSecret {
                owner: SecretOwner::Session(session.id),
                kind: SecretKind::Password,
                value: Zeroizing::new("s3cret".to_string()),
            }]),
        };
        let json = serde_json::to_string(&document).unwrap();
        assert!(json.contains("s3cret"));

        let decoded: BackupDocument = serde_json::from_str(&json).unwrap();
        let secrets = decoded.secrets.expect("口令应当被读回");
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].kind, SecretKind::Password);
        assert_eq!(secrets[0].value.as_str(), "s3cret");

        let debug = format!("{:?}", secrets[0]);
        assert!(!debug.contains("s3cret"));
        assert!(debug.contains("已脱敏"));
    }

    /// 模板口令写 `identity_id`，老备份的 `session_id` 必须照常可读：
    /// 备份文件是用户手上的历史资产，格式变更不能让他们导不出旧包。
    #[test]
    fn identity_secrets_use_separate_field_and_legacy_still_parses() {
        let identity = crate::identity::Identity::new(
            "ops",
            "ops",
            crate::AuthMethod::Password { save: true },
        );
        let secret = BackupSecret {
            owner: SecretOwner::Identity(identity.id),
            kind: SecretKind::Password,
            value: Zeroizing::new("p@ss".to_string()),
        };
        let json = serde_json::to_string(&secret).unwrap();
        assert!(json.contains("identity_id"), "实际: {json}");
        assert!(!json.contains("session_id"), "实际: {json}");

        let decoded: BackupSecret = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.owner, SecretOwner::Identity(identity.id));

        let legacy = format!(
            "{{\"session_id\":\"{}\",\"kind\":\"password\",\"value\":\"old\"}}",
            crate::ids::SessionId::new()
        );
        let decoded: BackupSecret = serde_json::from_str(&legacy).unwrap();
        assert!(matches!(decoded.owner, SecretOwner::Session(_)));

        // 两个归属字段同时出现时按坏数据处理，绝不猜归属。
        let both = format!(
            "{{\"session_id\":\"{}\",\"identity_id\":\"{}\",\"kind\":\"password\",\"value\":\"x\"}}",
            crate::ids::SessionId::new(),
            identity.id
        );
        assert!(serde_json::from_str::<BackupSecret>(&both).is_err());
        let neither = r#"{"kind":"password","value":"x"}"#;
        assert!(serde_json::from_str::<BackupSecret>(neither).is_err());
    }

    /// 口令类型由认证方式决定：只有口令认证与私钥认证才有钥匙串条目。
    #[test]
    fn secret_kinds_follow_auth_method() {
        let password = session_with_password("web-01");
        assert_eq!(password.secret_kinds(), &[SecretKind::Password]);

        let key = Session::new(
            "web-02",
            crate::Connection::Ssh(crate::SshConnection {
                host: "example.com".into(),
                port: 22,
                username: "deploy".into(),
                auth: crate::AuthMethod::PrivateKey {
                    path: std::path::PathBuf::from("/home/deploy/.ssh/id_ed25519"),
                    cert_path: None,
                    save_passphrase: true,
                },
                options: Default::default(),
                proxy_jump: None,
            }),
        );
        assert_eq!(key.secret_kinds(), &[SecretKind::KeyPassphrase]);

        let local = Session::new("local", crate::Connection::Local(Default::default()));
        assert!(local.secret_kinds().is_empty());
    }
}
