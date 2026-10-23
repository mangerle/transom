//! Persistence for sessions, secrets, settings, and trusted host keys.

mod file_io;

pub mod backup;
pub mod known_hosts;
pub mod paths;
pub mod secrets;
pub mod sessions;
pub mod settings;

pub use backup::{
    BackupFormat, EncryptedBackupService, decrypt_catalog, encrypt_catalog, read_encrypted_backup,
    write_encrypted_backup,
};
pub use known_hosts::JsonKnownHostsStore;
pub use secrets::KeyringSecretStore;
pub use sessions::JsonSessionStore;
pub use settings::JsonSettingsStore;
