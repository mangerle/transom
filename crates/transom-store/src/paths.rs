use directories::BaseDirs;
use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("transom"))
        .unwrap_or_else(|| PathBuf::from(".transom"))
}

pub fn known_hosts_path() -> PathBuf {
    config_dir().join("known_hosts.json")
}

pub fn sessions_path() -> PathBuf {
    config_dir().join("sessions.json")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}
