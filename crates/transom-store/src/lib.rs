//! Persistence for sessions, secrets, settings, and trusted host keys.

mod file_io;
mod history;

pub mod backup;
pub mod desktop;
pub mod known_hosts;
pub mod paths;
pub mod secrets;
pub mod sessions;
pub mod settings;

// 备份/同步信封（BackupFormat、加解密入口）位于独立的 `transom_crypto` crate：
// 存储层只暴露"文件"侧能力。调用方（sync、store）统一从 transom_crypto 引用。
pub use backup::{CatalogBackupService, read_backup, write_backup};
pub use file_io::write_atomic;
pub use history::JsonHistoryStore;
pub use known_hosts::JsonKnownHostsStore;
pub use secrets::KeyringSecretStore;
pub use sessions::JsonSessionStore;
pub use settings::JsonSettingsStore;
