use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyInfo {
    pub algorithm: String,
    pub fingerprint: String,
    pub public_key: Option<String>,
}

impl HostKeyInfo {
    pub fn new(
        algorithm: impl Into<String>,
        fingerprint: impl Into<String>,
        public_key: Option<String>,
    ) -> Self {
        Self {
            algorithm: algorithm.into(),
            fingerprint: fingerprint.into(),
            public_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    Match,
    Unknown,
    Mismatch { previous_fingerprint: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyRecord {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub public_key: Option<String>,
}

pub trait KnownHostsStore: Send + Sync {
    fn check(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<HostKeyStatus, AppError>;
    fn record(&self, host: &str, port: u16, info: &HostKeyInfo) -> Result<(), AppError>;
    fn remove(&self, host: &str, port: u16) -> Result<(), AppError>;
    fn list(&self) -> Result<Vec<HostKeyRecord>, AppError>;
}

pub fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}
