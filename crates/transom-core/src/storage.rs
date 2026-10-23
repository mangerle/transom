use crate::catalog::SessionCatalog;
use crate::error::AppError;
use crate::secrets::SecretKey;
use crate::settings::AppSettings;
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

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError>;
    fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError>;
    fn delete(&self, key: &SecretKey) -> Result<(), AppError>;
}
