//! 云端同步的端点、通道选择与偏好设置。
//!
//! 设计原理（Why）：
//! - WebDAV 与 S3 各自保存独立配置，切换通道不会清空另一侧拓扑；
//! - 启用状态与当前通道分离，暂停同步不删除配置，也不会误触发网络动作；
//! - 设备身份、自动策略与同步基线共同描述本机同步状态，供侧边栏与历史使用。
//!
//! 代价与局限：旧设置文件使用单个 `endpoint` 字段，仍需保留迁移读取；
//! `legacy_endpoint` 只参与反序列化，归一化后立即转成新配置。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod git;
pub use git::GitSyncSettings;

/// 界面当前正在编辑和使用的同步通道。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncChannel {
    #[default]
    Off,
    Webdav,
    S3,
    Git,
}

impl SyncChannel {
    /// 稳定的界面键。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Webdav => "webdav",
            Self::S3 => "s3",
            Self::Git => "git",
        }
    }
}

/// 云端同步端点拓扑。凭据不入此结构，由实现层从钥匙串读取。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SyncEndpoint {
    /// WebDAV 端点（坚果云、Nextcloud、Infomaniak 等）。
    Webdav {
        url: String,
        username: String,
        directory: String,
    },
    /// S3 兼容端点（AWS S3、MinIO、Cloudflare R2 等）。
    S3 {
        endpoint: String,
        region: String,
        bucket: String,
        access_key: String,
        directory: String,
        #[serde(default)]
        virtual_host_style: bool,
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

    /// 端点类型的稳定键，用于同步历史与状态展示。
    pub const fn provider_key(&self) -> &'static str {
        match self {
            SyncEndpoint::Webdav { .. } => "webdav",
            SyncEndpoint::S3 { .. } => "s3",
        }
    }

    /// 归一化端点配置：去空白、丢弃空段与 `.` 段；非法配置返回 `None`。
    pub fn sanitized(self) -> Option<Self> {
        match self {
            SyncEndpoint::Webdav {
                url,
                username,
                directory,
            } => WebdavSyncSettings {
                url,
                username,
                directory,
            }
            .sanitized()
            .map(|settings| settings.to_endpoint()),
            SyncEndpoint::S3 {
                endpoint,
                region,
                bucket,
                access_key,
                directory,
                virtual_host_style,
            } => S3SyncSettings {
                endpoint,
                region,
                bucket,
                access_key,
                directory,
                virtual_host_style,
            }
            .sanitized()
            .map(|settings| settings.to_endpoint()),
        }
    }
}

/// WebDAV 独立配置；空字段由归一化流程拒绝上网。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebdavSyncSettings {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub directory: String,
}

impl WebdavSyncSettings {
    /// 校验并生成传输层端点。
    pub fn sanitized(self) -> Option<Self> {
        let url = self.url.trim().trim_end_matches('/').to_string();
        let username = self.username.trim().to_string();
        let directory = clean_directory(&self.directory)?;
        (url.starts_with("http://") || url.starts_with("https://")).then_some(Self {
            url,
            username,
            directory,
        })
    }

    /// 转为传输层统一端点。
    pub fn to_endpoint(&self) -> SyncEndpoint {
        SyncEndpoint::Webdav {
            url: self.url.clone(),
            username: self.username.clone(),
            directory: self.directory.clone(),
        }
    }
}

/// S3 独立配置；Session Token 与 Secret Key 只存在钥匙串。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3SyncSettings {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub access_key: String,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub virtual_host_style: bool,
}

impl S3SyncSettings {
    /// 校验并生成传输层端点。
    pub fn sanitized(self) -> Option<Self> {
        let endpoint = self.endpoint.trim().trim_end_matches('/').to_string();
        let region = self.region.trim().to_string();
        let bucket = self.bucket.trim().to_string();
        let access_key = self.access_key.trim().to_string();
        let directory = clean_directory(&self.directory)?;
        let valid = (endpoint.starts_with("http://") || endpoint.starts_with("https://"))
            && !region.is_empty()
            && !bucket.is_empty()
            && !access_key.is_empty();
        valid.then_some(Self {
            endpoint,
            region,
            bucket,
            access_key,
            directory,
            virtual_host_style: self.virtual_host_style,
        })
    }

    /// 转为传输层统一端点。
    pub fn to_endpoint(&self) -> SyncEndpoint {
        SyncEndpoint::S3 {
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            bucket: self.bucket.clone(),
            access_key: self.access_key.clone(),
            directory: self.directory.clone(),
            virtual_host_style: self.virtual_host_style,
        }
    }
}

/// 远端目录的段数上限：防止误填超长路径，也呼应部分对象存储的键长约束。
const MAX_DIRECTORY_SEGMENTS: usize = 8;

fn clean_directory(directory: &str) -> Option<String> {
    let segments: Vec<&str> = directory
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.contains(&"..") || segments.len() > MAX_DIRECTORY_SEGMENTS {
        return None;
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// 单个提供商的最近同步基线与时间。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBaseline {
    #[serde(default)]
    pub last_push_at: Option<i64>,
    #[serde(default)]
    pub last_pull_at: Option<i64>,
    #[serde(default)]
    pub last_checked_at: Option<i64>,
    #[serde(default)]
    pub last_sync_catalog_hash: Option<String>,
    #[serde(default)]
    pub last_remote_hash: Option<String>,
}

impl SyncBaseline {
    /// 是否存在至少一项可迁移或可展示的基线信息。
    pub fn has_data(&self) -> bool {
        self.last_push_at.is_some()
            || self.last_pull_at.is_some()
            || self.last_checked_at.is_some()
            || self.last_sync_catalog_hash.is_some()
            || self.last_remote_hash.is_some()
    }
}

/// 云同步偏好与最近一次同步基线。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettings {
    /// 云同步设置结构版本；旧配置为 0，用于一次性迁移通道与认证槽位。
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub device_id: String,
    #[serde(default = "default_device_name")]
    pub device_name: String,
    /// 当前界面编辑与使用的通道。
    #[serde(default)]
    pub active_channel: SyncChannel,
    /// WebDAV 配置与当前通道无关，切换回来时原样恢复。
    #[serde(default)]
    pub webdav: WebdavSyncSettings,
    /// S3 配置与当前通道无关，切换回来时原样恢复。
    #[serde(default)]
    pub s3: S3SyncSettings,
    #[serde(default)]
    pub git: Option<GitSyncSettings>,
    /// 旧版单端点字段，仅用于迁移读取。
    #[serde(default, rename = "endpoint", skip_serializing)]
    pub legacy_endpoint: Option<SyncEndpoint>,
    #[serde(default = "default_include_secrets")]
    pub include_secrets: bool,
    #[serde(default)]
    pub auto_check_on_startup: bool,
    #[serde(default)]
    pub auto_push_on_change: bool,
    #[serde(default)]
    pub auto_pull_remote_changes: bool,
    #[serde(default = "default_auto_push_debounce_seconds")]
    pub auto_push_debounce_seconds: u64,
    /// 按提供商保存的最近同步基线与时间。
    #[serde(default)]
    pub baselines: BTreeMap<String, SyncBaseline>,
    /// 旧版全局时间字段，仅在归一化时迁入当前提供商基线。
    #[serde(default, skip_serializing)]
    pub last_push_at: Option<i64>,
    #[serde(default, skip_serializing)]
    pub last_pull_at: Option<i64>,
    #[serde(default, skip_serializing)]
    pub last_checked_at: Option<i64>,
    #[serde(default, skip_serializing)]
    pub last_sync_catalog_hash: Option<String>,
    #[serde(default, skip_serializing)]
    pub last_remote_hash: Option<String>,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: false,
            device_id: String::new(),
            device_name: default_device_name(),
            active_channel: SyncChannel::Off,
            webdav: WebdavSyncSettings::default(),
            s3: S3SyncSettings::default(),
            git: None,
            legacy_endpoint: None,
            include_secrets: true,
            auto_check_on_startup: false,
            auto_push_on_change: false,
            auto_pull_remote_changes: false,
            auto_push_debounce_seconds: default_auto_push_debounce_seconds(),
            baselines: BTreeMap::new(),
            last_push_at: None,
            last_pull_at: None,
            last_checked_at: None,
            last_sync_catalog_hash: None,
            last_remote_hash: None,
        }
    }
}

impl SyncSettings {
    /// 写盘前归一化：迁移旧端点、清洗两份配置并生成稳定设备 ID。
    pub fn normalize(&mut self) {
        let legacy_schema = self.schema_version == 0;
        if let Some(endpoint) = self.legacy_endpoint.take() {
            match endpoint {
                SyncEndpoint::Webdav {
                    url,
                    username,
                    directory,
                } => {
                    if self.webdav.url.trim().is_empty() {
                        self.webdav = WebdavSyncSettings {
                            url,
                            username,
                            directory,
                        };
                    }
                    if self.active_channel == SyncChannel::Off {
                        self.active_channel = SyncChannel::Webdav;
                    }
                }
                SyncEndpoint::S3 {
                    endpoint,
                    region,
                    bucket,
                    access_key,
                    directory,
                    virtual_host_style,
                } => {
                    if self.s3.endpoint.trim().is_empty() {
                        self.s3 = S3SyncSettings {
                            endpoint,
                            region,
                            bucket,
                            access_key,
                            directory,
                            virtual_host_style,
                        };
                    }
                    if self.active_channel == SyncChannel::Off {
                        self.active_channel = SyncChannel::S3;
                    }
                }
            }
        }
        if legacy_schema && self.active_channel == SyncChannel::Off && self.git.is_some() {
            self.active_channel = SyncChannel::Git;
        }
        self.schema_version = 1;
        self.webdav = self.webdav.clone().sanitized().unwrap_or_default();
        self.s3 = self.s3.clone().sanitized().unwrap_or_default();
        if let Some(git) = self.git.take() {
            self.git = git.sanitized();
        }
        let legacy = SyncBaseline {
            last_push_at: self.last_push_at.take(),
            last_pull_at: self.last_pull_at.take(),
            last_checked_at: self.last_checked_at.take(),
            last_sync_catalog_hash: self.last_sync_catalog_hash.take(),
            last_remote_hash: self.last_remote_hash.take(),
        };
        if legacy.has_data()
            && let Some(provider) = self.active_provider_key()
        {
            self.baselines.entry(provider.to_string()).or_insert(legacy);
        }
        self.device_id = self.device_id.trim().to_string();
        if self.device_id.is_empty() {
            self.device_id = uuid::Uuid::new_v4().to_string();
        }
        self.device_name = self.device_name.trim().to_string();
        if self.device_name.is_empty() {
            self.device_name = default_device_name();
        }
        self.auto_push_debounce_seconds = self.auto_push_debounce_seconds.clamp(5, 3600);
    }

    /// 当前通道对应的 WebDAV/S3 传输端点。
    pub fn active_endpoint(&self) -> Option<SyncEndpoint> {
        match self.active_channel {
            SyncChannel::Webdav => self.webdav.clone().sanitized().map(|v| v.to_endpoint()),
            SyncChannel::S3 => self.s3.clone().sanitized().map(|v| v.to_endpoint()),
            SyncChannel::Off | SyncChannel::Git => None,
        }
    }

    /// 当前通道的稳定键；Git 也使用独立基线记录最近成功时间。
    pub fn active_provider_key(&self) -> Option<&'static str> {
        match self.active_channel {
            SyncChannel::Webdav => Some("webdav"),
            SyncChannel::S3 => Some("s3"),
            SyncChannel::Git => Some("git"),
            SyncChannel::Off => None,
        }
    }

    /// 当前提供商的同步基线；未记录时返回默认值。
    pub fn active_baseline(&self) -> SyncBaseline {
        self.active_provider_key()
            .and_then(|provider| self.baselines.get(provider).cloned())
            .unwrap_or_default()
    }

    /// 取得当前提供商基线，供同步完成时原地更新。
    pub fn active_baseline_mut(&mut self) -> Option<&mut SyncBaseline> {
        let provider = self.active_provider_key()?;
        Some(self.baselines.entry(provider.to_string()).or_default())
    }

    /// 当前通道是否指向 WebDAV/S3。
    pub fn has_endpoint(&self) -> bool {
        self.active_endpoint().is_some()
    }

    /// 是否应允许 WebDAV/S3 同步动作。
    pub fn endpoint_enabled(&self) -> bool {
        self.enabled && self.has_endpoint()
    }

    /// 是否应允许 Git 同步动作。
    pub fn git_enabled(&self) -> bool {
        self.enabled && self.active_channel == SyncChannel::Git && self.git.is_some()
    }
}

fn default_include_secrets() -> bool {
    true
}

fn default_auto_push_debounce_seconds() -> u64 {
    60
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "本机".to_string())
}
