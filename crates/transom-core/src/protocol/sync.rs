//! 云端同步契约：端点配置模型、运行状态与远端存储抽象接口。
//!
//! 设计取舍（Why）：
//! - 同步主体仍是加密备份对象，但围绕它增加了设备身份、远端检查、自动策略
//!   与可持久化活动结果；这些信息让侧边栏能展示真实状态，而不是猜测设置值；
//! - 端点凭据与同步口令绝不进入设置文件，秘密统一进系统钥匙串；
//! - 常规上传返回合并后的完整目录，避免"远端已合并、本机仍落后"却显示成功；
//!   显式冲突处理再通过强制本地/强制远端模式绕过自动合并。

use crate::backup::BackupDocument;
use crate::error::{AppError, ErrorCode};
use crate::model::catalog::SessionCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 远端固定对象名：目录下的唯一备份文件。
///
/// 文件名自带 `.transombak` 后缀，便于用户在坚果云/Nextcloud 网页里一眼
/// 识别来源，也避免和用户自己放在同步目录里的文件重名。
pub const SYNC_OBJECT_NAME: &str = "catalog.transombak";

/// 云端同步的远端对象字节上限，与会话备份文件的本地上限一致。
///
/// 备份仅含配置文本，16 MiB 足以容纳数万条会话；下载侧按同一上限拒绝
/// 恶意或异常的超大响应，防止把整份内存交给不可信端点。
pub const SYNC_MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

/// 触发一次同步动作的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncTrigger {
    /// 用户在设置页或同步侧边栏显式触发。
    Manual,
    /// 应用启动后的自动检查。
    Startup,
    /// 周期检查或由周期检查派生的动作。
    Periodic,
    /// 本地目录变化防抖到期。
    LocalChange,
    /// 用户在冲突预览中作出的显式选择。
    ConflictResolution,
    /// Git 仓库定时同步。
    GitInterval,
}

impl SyncTrigger {
    /// 稳定的活动键，日志与历史共用。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Startup => "startup",
            Self::Periodic => "periodic",
            Self::LocalChange => "local_change",
            Self::ConflictResolution => "conflict_resolution",
            Self::GitInterval => "git_interval",
        }
    }
}

/// 一次同步动作的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOperation {
    Check,
    TestConnection,
    Push,
    Pull,
    Git,
}

impl SyncOperation {
    /// 稳定的活动键，日志与历史共用。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::TestConnection => "test_connection",
            Self::Push => "push",
            Self::Pull => "pull",
            Self::Git => "git",
        }
    }
}

/// 控制器回传的活动元数据；UI 用同一结构生成历史与耗时。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncEventMeta {
    pub trigger: SyncTrigger,
    pub started_at_ms: i64,
    pub duration_ms: u64,
}

/// 目录快照的摘要，不携带任何口令或完整对象路径。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncDocumentSummary {
    pub sessions: usize,
    pub folders: usize,
    pub identities: usize,
    pub engines: usize,
    pub engine_folders: usize,
    pub secrets: usize,
}

impl SyncDocumentSummary {
    /// 从目录生成摘要。
    pub fn from_catalog(catalog: &SessionCatalog) -> Self {
        Self {
            sessions: catalog.sessions.len(),
            folders: catalog.folders.len(),
            identities: catalog.identities.len(),
            engines: catalog.engines.len(),
            engine_folders: catalog.engine_folders.len(),
            secrets: 0,
        }
    }

    /// 从备份文档生成摘要；口令只计数，不保留内容。
    pub fn from_document(document: &BackupDocument) -> Self {
        Self {
            sessions: document.sessions.len(),
            folders: document.folders.len(),
            identities: document.identities.len(),
            engines: document.engines.len(),
            engine_folders: document.engine_folders.len(),
            secrets: document.secrets.as_ref().map_or(0, Vec::len),
        }
    }
}

/// 本机与远端的相对变化关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncRelation {
    /// 两端都与最近成功基线一致。
    UpToDate,
    /// 只有本机存在未同步修改，或远端没有备份。
    LocalAhead,
    /// 只有远端存在新版本。
    RemoteAhead,
    /// 两端都偏离基线，必须由用户明确选择。
    Diverged,
    /// 基线存在但远端对象被删除。
    RemoteMissing,
}

/// 远端检查结果；不携带远端文档内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCheckReport {
    pub provider: String,
    pub at: i64,
    pub duration_ms: u64,
    pub local_hash: String,
    pub remote_hash: Option<String>,
    pub remote_present: bool,
    pub local_changed: bool,
    pub remote_changed: bool,
    pub relation: SyncRelation,
    pub local_summary: SyncDocumentSummary,
    pub remote_summary: Option<SyncDocumentSummary>,
    pub remote_version: Option<String>,
}

/// 连通性测试结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTestReport {
    pub provider: String,
    pub at: i64,
    pub duration_ms: u64,
    pub remote_present: bool,
    pub remote_version: Option<String>,
}

/// 上传成功后的结果；携带合并目录，界面必须回填到本地状态。
#[derive(Debug)]
pub struct SyncPushReport {
    /// 实际写入远端、并应成为本机新状态的目录。
    pub catalog: SessionCatalog,
    pub sessions: usize,
    pub folders: usize,
    /// 加密进备份的钥匙串口令数量。
    pub secrets: usize,
    /// 加密信封的字节数。
    pub bytes: u64,
    /// 完成时刻的 Unix 时间戳（秒），由控制层回填到设置。
    pub at: i64,
    /// 本机目录指纹，用于下一次自动同步判断。
    pub catalog_hash: String,
    /// 远端加密对象指纹。
    pub remote_hash: String,
}

/// 下载成功后的结果；口令随文档交回 UI 恢复流程。
#[derive(Debug)]
pub struct SyncPullReport {
    pub document: BackupDocument,
    pub remote_version: Option<String>,
    pub remote_hash: String,
    pub bytes: u64,
    pub at: i64,
}

/// 冲突解决时上传本机版本，还是执行常规双向合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudPushMode {
    Merge,
    ForceLocal,
}

/// 冲突解决时以远端整体替换，还是执行常规合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudPullMode {
    Merge,
    ForceRemote,
}

/// Git 同步完成后回传给界面的结果。
#[derive(Debug, Clone)]
pub struct GitSyncResult {
    /// 合并后的完整目录，界面应整体替换本地目录。
    pub catalog: SessionCatalog,
    pub report: crate::model::catalog::SyncMergeReport,
    pub pushed: bool,
    pub bytes: u64,
    pub at: i64,
}

/// 云端同步的远端存储抽象：读写固定对象并探测远端状态。
///
/// 接口刻意不提供目录枚举：同步协议只需要一个对象与它的版本标识，能力越窄，
/// WebDAV 与 S3 的行为差异面越小，也避免对象存储列出整个桶带来的权限要求。
#[async_trait::async_trait]
pub trait CloudSyncTarget: Send + Sync {
    /// 把加密备份上传到远端固定对象，目录不存在时自动逐级创建。
    ///
    /// # Errors
    /// 网络失败、认证失败、远端拒绝写入或响应超时时返回 `AppError`。
    async fn put_document(&self, data: &[u8]) -> Result<(), AppError>;

    /// 下载远端固定对象的完整内容；远端还没有备份时报错而非空数据。
    ///
    /// # Errors
    /// 对象不存在、网络失败、认证失败、响应超过大小上限时返回 `AppError`。
    async fn get_document(&self) -> Result<Vec<u8>, AppError>;

    /// 下载内容并返回版本标识（WebDAV/S3 的 ETag）；不支持版本控制的
    /// 实现复用普通下载并返回 `None`。
    ///
    /// # Errors
    /// 与 [`Self::get_document`] 相同。
    async fn get_document_versioned(&self) -> Result<(Vec<u8>, Option<String>), AppError> {
        Ok((self.get_document().await?, None))
    }

    /// 只探测对象是否存在并返回版本标识，不下载正文。
    ///
    /// 默认实现退回完整下载，仅供不支持 HEAD 的测试目标使用。
    ///
    /// # Errors
    /// 网络失败或认证失败时返回 `AppError`；对象不存在返回 `Ok(None)`。
    async fn probe_document(&self) -> Result<Option<String>, AppError> {
        Ok(Some(self.get_document_versioned().await?.1).flatten())
    }

    /// 带前置版本条件上传：远端版本不等于 `version` 时返回"版本冲突"错误。
    ///
    /// 不支持条件写入的实现退回普通上传；调用方仍能拿到加密备份，
    /// 只是失去并发保护。
    ///
    /// # Errors
    /// 版本冲突、网络失败、认证失败或远端拒绝写入时返回 `AppError`。
    async fn put_document_if_match(
        &self,
        data: &[u8],
        version: Option<&str>,
    ) -> Result<(), AppError> {
        let _ = version;
        self.put_document(data).await
    }
}

/// 计算目录稳定指纹：按领域 JSON 序列化后的 SHA-256。
///
/// # Errors
/// 目录无法序列化时返回 `CloudSyncFailed`；正常情况下该错误不可达，但公开
/// 契约仍显式返回错误，避免内部实现被误认为数学上的绝对成功。
pub fn sync_catalog_hash(catalog: &SessionCatalog) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(catalog).map_err(|error| {
        AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("计算目录指纹失败: {error}"),
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// 判断错误是否为"远端已被其他设备修改"的版本冲突。
pub fn is_sync_conflict(error: &AppError) -> bool {
    error.code == ErrorCode::CloudSyncConflict
}

/// 供测试与"先合并后落盘"流程快速构造的文档别名，无附加字段。
pub type SyncPullDocument = BackupDocument;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::settings::{AppSettings, SyncEndpoint};

    #[test]
    fn sync_settings_load_from_legacy_config() {
        let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert!(settings.sync.active_endpoint().is_none());
        assert!(settings.sync.include_secrets);
        assert!(settings.sync.active_baseline().last_push_at.is_none());
        assert!(!settings.sync.enabled);
        assert!(!settings.sync.auto_push_on_change);
        assert_eq!(settings.sync.auto_push_debounce_seconds, 60);
    }

    #[test]
    fn legacy_single_endpoint_migrates_to_provider_specific_config() {
        let mut settings: AppSettings = serde_json::from_str(
            r#"{
                "sync": {
                    "enabled": true,
                    "last_push_at": 123,
                    "endpoint": {
                        "kind": "webdav",
                        "url": "https://dav.example.com",
                        "username": "user",
                        "directory": "transom"
                    },
                    "git": {
                        "remote": "git@example.com:repo.git",
                        "branch": "sync",
                        "interval_minutes": 5
                    }
                }
            }"#,
        )
        .unwrap();
        settings.sync.normalize();
        assert_eq!(settings.sync.active_channel, crate::SyncChannel::Webdav);
        assert_eq!(settings.sync.webdav.username, "user");
        assert!(settings.sync.git.is_some());
        assert!(settings.sync.active_endpoint().is_some());
        assert!(settings.sync.legacy_endpoint.is_none());
        assert_eq!(settings.sync.active_baseline().last_push_at, Some(123));
    }

    #[test]
    fn legacy_git_only_config_migrates_to_git_channel() {
        let mut settings: AppSettings = serde_json::from_str(
            r#"{
                "sync": {
                    "git": {
                        "remote": "git@example.com:repo.git",
                        "branch": "sync",
                        "interval_minutes": 5
                    }
                }
            }"#,
        )
        .unwrap();
        settings.sync.normalize();
        assert_eq!(settings.sync.active_channel, crate::SyncChannel::Git);
        assert!(
            !settings.sync.git_enabled(),
            "旧配置默认暂停，避免升级后立即联网"
        );
        assert_eq!(settings.sync.schema_version, 1);
    }

    #[test]
    fn provider_specific_configs_survive_channel_switches() {
        let mut sync = crate::SyncSettings {
            active_channel: crate::SyncChannel::Webdav,
            webdav: crate::WebdavSyncSettings {
                url: "https://dav.example.com".into(),
                username: "user".into(),
                directory: "webdav-root".into(),
            },
            s3: crate::S3SyncSettings {
                endpoint: "https://s3.example.com".into(),
                region: "us-east-1".into(),
                bucket: "vault".into(),
                access_key: "AKID".into(),
                directory: "s3-root".into(),
                virtual_host_style: true,
            },
            ..Default::default()
        };
        sync.normalize();
        assert!(matches!(
            sync.active_endpoint(),
            Some(SyncEndpoint::Webdav { .. })
        ));
        sync.active_channel = crate::SyncChannel::S3;
        assert!(matches!(
            sync.active_endpoint(),
            Some(SyncEndpoint::S3 {
                virtual_host_style: true,
                ..
            })
        ));
        sync.active_channel = crate::SyncChannel::Webdav;
        assert_eq!(sync.webdav.directory, "webdav-root");
        assert_eq!(sync.s3.directory, "s3-root");
    }

    #[test]
    fn endpoint_round_trips_with_kind_tag() {
        let endpoint = SyncEndpoint::Webdav {
            url: "https://dav.jianguoyun.com/dav".into(),
            username: "user".into(),
            directory: "transom".into(),
        };
        let json = serde_json::to_string(&endpoint).unwrap();
        assert!(json.contains("\"kind\":\"webdav\""));
        let decoded: SyncEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, endpoint);
    }

    #[test]
    fn sanitize_trims_and_rejects_invalid_endpoints() {
        let endpoint = SyncEndpoint::Webdav {
            url: "  https://dav.example.com/dav/  ".into(),
            username: " user ".into(),
            directory: "a/./b/c".into(),
        };
        let SyncEndpoint::Webdav {
            url,
            username,
            directory,
        } = endpoint.sanitized().unwrap()
        else {
            unreachable!()
        };
        assert_eq!(url, "https://dav.example.com/dav");
        assert_eq!(username, "user");
        assert_eq!(directory, "a/b/c");

        let missing_scheme = SyncEndpoint::Webdav {
            url: "dav.example.com".into(),
            username: "u".into(),
            directory: "t".into(),
        };
        assert!(missing_scheme.sanitized().is_none());

        let no_bucket = SyncEndpoint::S3 {
            endpoint: "https://s3.example.com".into(),
            region: "us-east-1".into(),
            bucket: "  ".into(),
            access_key: "AKID".into(),
            directory: "t".into(),
            virtual_host_style: false,
        };
        assert!(no_bucket.sanitized().is_none());
    }

    #[test]
    fn sanitize_drops_traversal_segments() {
        let endpoint = SyncEndpoint::Webdav {
            url: "https://dav.example.com".into(),
            username: "u".into(),
            directory: "../etc".into(),
        };
        assert!(endpoint.sanitized().is_none());
        // 中间的 ".." 同样拒绝：只拦开头会留下 "a/../../b" 这类越界路径。
        let nested = SyncEndpoint::Webdav {
            url: "https://dav.example.com".into(),
            username: "u".into(),
            directory: "a/b/../../c".into(),
        };
        assert!(nested.sanitized().is_none());
    }

    #[test]
    fn sync_conflict_is_decided_by_error_code() {
        assert!(is_sync_conflict(&AppError::new(
            ErrorCode::CloudSyncConflict
        )));
        // 普通同步失败即便文案提到"版本冲突"也不能被误判：冲突要触发
        // 拉取合并重试，误判会把网络故障拖进重试循环。
        assert!(!is_sync_conflict(&AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            "版本冲突字样不应参与判定",
        )));
        assert!(!is_sync_conflict(&AppError::new(
            ErrorCode::CloudSyncMissing
        )));
    }
}
