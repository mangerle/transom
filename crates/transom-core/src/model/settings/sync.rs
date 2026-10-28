//! 云端同步的端点与偏好设置。
//!
//! 设计原理（Why）：这些类型描述"用户配置了什么"，属于领域模型而非线格式；
//! 放在 settings 子域后，设置模型不再反向依赖 protocol，同步传输层只保留
//! 远端存储抽象与结果类型。
//!
//! 代价与局限：SyncEndpoint 同时参与设置归一化与传输目标构造，迁移后两处
//! 都引用本模块；后续若出现独立的远端拓扑类型，应在此拆分而不是加可选字段。

use serde::{Deserialize, Serialize};

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
    /// 私有 Git 仓库同步配置；`None` 表示未启用。
    ///
    /// 与 WebDAV/S3 端点互不排斥：用户可以同时配置两条通道，各自独立触发。
    #[serde(default)]
    pub git: Option<GitSyncSettings>,
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
            git: None,
            include_secrets: true,
            last_push_at: None,
            last_pull_at: None,
        }
    }
}

/// 私有 Git 仓库同步配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSyncSettings {
    /// 远端地址：SSH（`git@host:repo.git`）或 HTTPS。
    pub remote: String,
    /// 专用分支，默认 `transom-sync`；与用户自己的分支隔离。
    #[serde(default = "default_git_branch")]
    pub branch: String,
    /// 自动同步间隔（分钟）；`0` 表示仅手动触发。
    #[serde(default)]
    pub interval_minutes: u64,
}

impl Default for GitSyncSettings {
    fn default() -> Self {
        Self {
            remote: String::new(),
            branch: default_git_branch(),
            interval_minutes: 0,
        }
    }
}

fn default_git_branch() -> String {
    "transom-sync".to_string()
}

impl GitSyncSettings {
    /// 归一化并校验：远端为空或分支名非法时返回 `None`。
    pub fn sanitized(self) -> Option<Self> {
        let remote = self.remote.trim().to_string();
        let branch = self.branch.trim().to_string();
        let branch = if branch.is_empty() {
            default_git_branch()
        } else {
            branch
        };
        let valid_branch = !branch.starts_with('-')
            && branch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'));
        (!remote.is_empty() && valid_branch).then_some(Self {
            remote,
            branch,
            interval_minutes: self.interval_minutes.min(7 * 24 * 60),
        })
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
        if let Some(git) = self.git.take() {
            self.git = git.sanitized();
        }
    }
}

fn default_include_secrets() -> bool {
    true
}
