use crate::error::AppError;
use crate::secrets::SecretKey;
use crate::session::Session;
use crate::settings::AppSettings;
use zeroize::Zeroizing;

pub trait SessionStore: Send + Sync {
    fn load(&self) -> Result<Vec<Session>, AppError>;
    fn save(&self, sessions: &[Session]) -> Result<(), AppError>;
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
