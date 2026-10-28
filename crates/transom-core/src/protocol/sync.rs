//! 云端同步契约：端点配置模型与远端存储的抽象接口。
//!
//! 设计取舍（Why）：
//! - 同步走"加密备份上云"而不是增量同步：会话目录体量小（数万条会话也在
//!   MiB 量级），全量推送简单可靠，增量合并的状态机复杂度不成比例；
//! - 端点凭据（WebDAV 密码、S3 Secret Key）与同步口令绝不进入设置文件，
//!   设置里只留非敏感的端点拓扑信息，秘密统一进系统钥匙串；
//! - 合并策略：会话按 updated_at 较新者胜，无时间戳的文件夹/凭据按操作
//!   方向裁决；下载合并前由界面在本机留下安全副本兜底。

use crate::backup::BackupDocument;
use crate::error::{AppError, ErrorCode};
use crate::model::catalog::SessionCatalog;
use serde::{Deserialize, Serialize};

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

/// 上传成功后的统计快照，用于界面回报与时间戳回填。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPushReport {
    pub sessions: usize,
    pub folders: usize,
    /// 加密进备份的钥匙串口令数量。
    pub secrets: usize,
    /// 加密信封的字节数。
    pub bytes: u64,
    /// 完成时刻的 Unix 时间戳（秒），由控制层回填到设置。
    pub at: i64,
}

/// 云端同步的远端存储抽象：上传或下载固定对象。
///
/// 为什么只有两个动作：同步协议刻意收敛为"单对象全量推送/拉取"，目录
/// 列举、删除等能力是备份服务的职责外溢，接口越窄，WebDAV 与 S3 两种
/// 实现的行为差异面就越小。
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
        assert!(settings.sync.endpoint.is_none());
        assert!(settings.sync.include_secrets);
        assert!(settings.sync.last_push_at.is_none());
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
