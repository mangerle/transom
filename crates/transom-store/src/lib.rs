//! Persistence for sessions, secrets, settings, and trusted host keys.

mod api_projects;
mod error;
mod file_io;
mod git_projects;
mod history;
mod registry_secrets;
pub mod shelf;

pub mod ai_store;
pub mod backup;
pub mod known_hosts;
pub mod paths;
pub mod secrets;
pub mod sessions;
pub mod settings;

// 备份/同步信封（BackupFormat、加解密入口）位于独立的 `transom_crypto` crate：
// 存储层只暴露"文件"侧能力。调用方（sync、store）统一从 transom_crypto 引用。
pub use ai_store::AiStore;
pub use api_projects::JsonApiProjectStore;
pub use backup::{CatalogBackupService, read_backup, write_backup};
pub use error::StorageError;
pub use file_io::{LocalTextError, read_utf8_limited, write_atomic, write_utf8_checked_replace};
pub use git_projects::JsonGitProjectStore;
pub use history::JsonHistoryStore;
pub use known_hosts::JsonKnownHostsStore;
pub use registry_secrets::hydrate_registry_passwords;
pub use secrets::KeyringSecretStore;
pub use sessions::JsonSessionStore;
pub use settings::JsonSettingsStore;
pub use shelf::{load_project_shelves, save_project_shelves};
