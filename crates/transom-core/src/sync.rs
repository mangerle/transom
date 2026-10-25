//! 云端同步契约：端点配置模型与远端存储的抽象接口。
//!
//! 设计取舍（Why）：
//! - 同步走"加密备份上云"而不是增量同步：会话目录体量小（数万条会话也在
//!   MiB 量级），全量推送简单可靠，增量合并的状态机复杂度不成比例；
//! - 端点凭据（WebDAV 密码、S3 Secret Key）与同步口令绝不进入设置文件，
//!   设置里只留非敏感的端点拓扑信息，秘密统一进系统钥匙串；
//! - 冲突策略是"后写者胜"：不做服务端合并，下载合并前由界面在本机留下
//!   安全副本兜底，避免多端并发写入造成不可逆丢失。

use crate::backup::BackupDocument;
use crate::error::AppError;
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

/// 云端同步端点拓扑。凭据不入此结构，由实现层从钥匙串读取。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SyncEndpoint {
    /// WebDAV 端点（坚果云、Nextcloud、Infomaniak 等）。
    ///
    /// `url` 是目录级根地址（如 `https://dav.jianguoyun.com/dav/`），
    /// `directory` 是其下用于存放备份的子目录（逐级自动创建）。
    Webdav {
        url: String,
        username: String,
        directory: String,
    },
    /// S3 兼容端点（AWS S3、MinIO、Cloudflare R2 等）。
    ///
    /// 统一使用 path-style 寻址（`{endpoint}/{bucket}/{key}`）：自托管
    /// MinIO 默认即 path-style，AWS 全区域仍支持，虚拟主机式寻址留待
    /// 有需求时再加，避免第一版就背上证书通配符的部署约束。
    S3 {
        /// 服务根地址，如 `https://s3.us-east-1.amazonaws.com`。
        endpoint: String,
        /// 区域标识；部分自托管实现校验不严格，但仍需一个合法区域串。
        region: String,
        bucket: String,
        access_key: String,
        directory: String,
    },
}

impl SyncEndpoint {
    /// 备份对象在远端的存放子目录（用户可见的拓扑信息）。
    pub fn directory(&self) -> &str {
        match self {
            SyncEndpoint::Webdav { directory, .. } | SyncEndpoint::S3 { directory, .. } => {
                directory
            }
        }
    }

    /// 归一化端点配置：去空白、丢弃空段与 `.` 段；非法配置返回 `None`。
    ///
    /// 设置文件属外部输入，未填主机应在保存时拒绝；目录含 `..` 段同样整条拒绝
    /// 而非静默洗掉——该目录会逐级拼到远端前缀上，留下 `..` 等于允许越出用户
    /// 配置的目录，洗掉又会让"界面上填的"与"实际同步到的位置"不一致。
    pub fn sanitized(self) -> Option<Self> {
        fn clean_directory(directory: &str) -> Option<String> {
            let segments: Vec<&str> = directory
                .split('/')
                .map(str::trim)
                .filter(|segment| !segment.is_empty() && *segment != ".")
                .collect();
            // 含 ".." 的目录整条判非法，而不是悄悄洗掉这一段：该目录会逐级拼到
            // 远端 WebDAV/S3 前缀上，留下 ".." 等于允许跳出用户配置的远端目录，
            // 静默丢弃又会让他填的路径与实际同步位置不一致（AGENTS.md 防穿越红线）。
            if segments.contains(&"..") || segments.len() > MAX_DIRECTORY_SEGMENTS {
                return None;
            }
            (!segments.is_empty()).then(|| segments.join("/"))
        }
        match self {
            SyncEndpoint::Webdav {
                url,
                username,
                directory,
            } => {
                let url = url.trim().trim_end_matches('/').to_string();
                let username = username.trim().to_string();
                let directory = clean_directory(&directory)?;
                (url.starts_with("http://") || url.starts_with("https://")).then_some(
                    SyncEndpoint::Webdav {
                        url,
                        username,
                        directory,
                    },
                )
            }
            SyncEndpoint::S3 {
                endpoint,
                region,
                bucket,
                access_key,
                directory,
            } => {
                let endpoint = endpoint.trim().trim_end_matches('/').to_string();
                let region = region.trim().to_string();
                let bucket = bucket.trim().to_string();
                let access_key = access_key.trim().to_string();
                let directory = clean_directory(&directory)?;
                let valid = (endpoint.starts_with("http://") || endpoint.starts_with("https://"))
                    && !region.is_empty()
                    && !bucket.is_empty()
                    && !access_key.is_empty();
                valid.then_some(SyncEndpoint::S3 {
                    endpoint,
                    region,
                    bucket,
                    access_key,
                    directory,
                })
            }
        }
    }
}

/// 远端目录的段数上限：防止误填超长路径，也呼应部分对象存储的键长约束。
const MAX_DIRECTORY_SEGMENTS: usize = 8;

/// 云端同步偏好：端点选择、口令快照携带与最近一次同步时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettings {
    /// `None` 表示未启用云同步。
    #[serde(default)]
    pub endpoint: Option<SyncEndpoint>,
    /// 是否把钥匙串里保存的会话口令一并加密上传。
    ///
    /// 默认开启：多设备同步的主要诉求就是"换机免重输密码"，关闭仅适合
    /// 把云端当作纯配置备份的用户；内容始终整体加密，开关不改变机密强度。
    #[serde(default = "default_include_secrets")]
    pub include_secrets: bool,
    /// 最近一次成功上传的 Unix 时间戳（秒）。
    #[serde(default)]
    pub last_push_at: Option<i64>,
    /// 最近一次成功下载合并的 Unix 时间戳（秒）。
    #[serde(default)]
    pub last_pull_at: Option<i64>,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            endpoint: None,
            include_secrets: true,
            last_push_at: None,
            last_pull_at: None,
        }
    }
}

impl SyncSettings {
    /// 写盘前归一化：端点配置洗成合法状态，非法端点整体置空。
    ///
    /// 为什么不保留非法端点：设置页的输入框回显来自端点字段，半残的端点
    /// 只会让下次打开时看到"看似填好实则不可用"的假象。
    pub fn normalize(&mut self) {
        if let Some(endpoint) = self.endpoint.take() {
            self.endpoint = endpoint.sanitized();
        }
    }
}

fn default_include_secrets() -> bool {
    true
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
}

/// 供测试与"先合并后落盘"流程快速构造的文档别名，无附加字段。
pub type SyncPullDocument = BackupDocument;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_settings_load_from_legacy_config() {
        let settings: crate::settings::AppSettings =
            serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
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
}
