use crate::backup::{
    BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions, BackupSecret,
    SecretRestoreReport,
};
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

/// 会话目录备份服务。
///
/// UI 只感知“导出/导入到某路径”，文件格式、是否加密与口令校验由实现层
/// 负责；这样测试中可注入内存实现，也避免界面 crate 直接依赖密码学库。
///
/// 加密与否由选项里有没有口令表达：明文备份是普通 JSON/YAML，加密备份是
/// 带口令信封的文件，两者互斥。
pub trait BackupService: Send + Sync {
    /// 将目录快照写入指定路径。
    ///
    /// # Errors
    /// 口令为空、钥匙串不可用、写盘失败或加密失败时返回 `AppError`。
    fn export(
        &self,
        path: &Path,
        catalog: &SessionCatalog,
        options: &BackupExportOptions,
    ) -> Result<BackupExportReport, AppError>;

    /// 从指定路径读取目录快照。
    ///
    /// # Errors
    /// 文件不可读、口令错误、格式不兼容或完整性校验失败时返回 `AppError`。
    fn import(
        &self,
        path: &Path,
        options: &BackupImportOptions,
    ) -> Result<BackupDocument, AppError>;

    /// 把备份携带的口令写回本机钥匙串。
    ///
    /// 本机已存在的同名口令一律保留，不做覆盖：导入备份不应改掉本机正在
    /// 使用的密码。
    ///
    /// # Errors
    /// 钥匙串整体不可用时返回 `AppError`；个别条目写入失败只计入统计。
    fn restore_secrets(&self, secrets: &[BackupSecret]) -> Result<SecretRestoreReport, AppError>;
}

/// 命令历史的读写契约。
///
/// 与 `SessionStore` 分开：历史是"用过什么"，会话目录是"有哪些机器"，
/// 体积与生命周期都不同，混在一个文件里会让每敲一条命令都重写整份会话库。
pub trait HistoryStore: Send + Sync {
    /// 读取历史；实现方对损坏与缺失一律降级成空表，不让应用起不来。
    fn load(&self) -> Vec<String>;

    /// 覆盖式保存整份历史。
    ///
    /// # Errors
    /// 目录创建、序列化或原子改名失败时返回底层错误；调用方（界面）只记提示，
    /// 写盘失败不影响本次会话的使用。
    fn save(&self, commands: &[String]) -> Result<(), AppError>;
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError>;
    fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError>;
    fn delete(&self, key: &SecretKey) -> Result<(), AppError>;
}
