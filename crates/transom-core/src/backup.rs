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
use crate::ids::SessionId;
use crate::secrets::SecretKind;
use crate::session::Session;
use serde::de::Deserializer;
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

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
            secrets: None,
        }
    }

    /// 拆出目录快照与口令列表，交接所有权避免口令被额外克隆。
    pub fn into_parts(self) -> (SessionCatalog, Option<Vec<BackupSecret>>) {
        (
            SessionCatalog {
                folders: self.folders,
                sessions: self.sessions,
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
        };
        catalog.normalize();
        self.folders = catalog.folders;
        self.sessions = catalog.sessions;
    }
}

/// 备份中的一条钥匙串口令。
pub struct BackupSecret {
    /// 口令归属的会话。
    pub session_id: SessionId,
    /// 口令种类：登录密码或私钥口令。
    pub kind: SecretKind,
    /// 口令明文；文档释放时归零。
    pub value: Zeroizing<String>,
}

impl fmt::Debug for BackupSecret {
    /// 口令值一律脱敏：备份文档会随错误信息与日志流转，明文不得外泄。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupSecret")
            .field("session_id", &self.session_id)
            .field("kind", &self.kind)
            .field("value", &"<已脱敏>")
            .finish()
    }
}

impl Serialize for BackupSecret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("BackupSecret", 3)?;
        state.serialize_field("session_id", &self.session_id)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("value", self.value.as_str())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BackupSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            session_id: SessionId,
            kind: SecretKind,
            value: String,
        }

        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            session_id: raw.session_id,
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
            secrets: Some(vec![BackupSecret {
                session_id: session.id,
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
            }),
        );
        assert_eq!(key.secret_kinds(), &[SecretKind::KeyPassphrase]);

        let local = Session::new("local", crate::Connection::Local(Default::default()));
        assert!(local.secret_kinds().is_empty());
    }
}
