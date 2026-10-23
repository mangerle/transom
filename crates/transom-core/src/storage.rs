use crate::catalog::SessionCatalog;
use crate::error::AppError;
use crate::secrets::SecretKey;
use crate::settings::AppSettings;
use std::path::Path;
use zeroize::Zeroizing;

/// 会话与文件夹的一次性完整读写契约。
///
/// 二者共享同一个 `sessions.json`，合并为一个快照可避免分别保存时出现
/// “文件夹已落盘、会话尚未落盘”的撕裂状态。
pub trait SessionStore: Send + Sync {
    fn load(&self) -> Result<SessionCatalog, AppError>;
    fn save(&self, catalog: &SessionCatalog) -> Result<(), AppError>;
}

pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Result<AppSettings, AppError>;
    fn save(&self, settings: &AppSettings) -> Result<(), AppError>;
}

/// 会话目录加密备份服务。
///
/// UI 只感知“导出/导入到某路径”，加密算法、文件格式与口令校验由实现层
/// 负责；这样测试中可注入内存实现，也避免界面 crate 直接依赖密码学库。
pub trait BackupService: Send + Sync {
    /// 将目录快照加密写入指定路径。
    ///
    /// # Errors
    /// 口令为空、写盘失败或加密失败时返回 `AppError`。
    fn export(
        &self,
        path: &Path,
        catalog: &SessionCatalog,
        passphrase: &str,
    ) -> Result<(), AppError>;

    /// 从指定路径读取并解密目录快照。
    ///
    /// # Errors
    /// 文件不可读、口令错误、格式不兼容或完整性校验失败时返回 `AppError`。
    fn import(&self, path: &Path, passphrase: &str) -> Result<SessionCatalog, AppError>;
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError>;
    fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError>;
    fn delete(&self, key: &SecretKey) -> Result<(), AppError>;
}
