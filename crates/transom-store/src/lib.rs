//! Persistence for sessions, secrets, settings, and trusted host keys.

mod file_io;
mod history;

pub mod backup;
pub mod desktop;
pub mod envelope;
pub mod known_hosts;
pub mod paths;
pub mod secrets;
pub mod sessions;
pub mod settings;

pub use backup::{
    BackupFormat, CatalogBackupService, decrypt_document, encrypt_document, read_backup,
    write_backup,
};
pub use envelope::{
    ENVELOPE_HEADER_LEN, ENVELOPE_MAGIC, ENVELOPE_VERSION, is_envelope, open, seal,
};
pub use history::JsonHistoryStore;
pub use known_hosts::JsonKnownHostsStore;
pub use secrets::KeyringSecretStore;
pub use sessions::JsonSessionStore;
pub use settings::JsonSettingsStore;
