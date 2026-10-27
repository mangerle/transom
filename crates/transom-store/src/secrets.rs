use keyring::Entry;
use transom_core::{AppError, ErrorCode, SecretKey, SecretStore};
use zeroize::Zeroizing;

pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &SecretKey) -> Result<Entry, AppError> {
        Entry::new(&self.service, &key.account()).map_err(|error| {
            AppError::with_detail(
                ErrorCode::SecretStoreUnavailable,
                format!("system keychain unavailable: {error}"),
            )
        })
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(AppError::with_detail(
                ErrorCode::SecretStoreUnavailable,
                format!("读取钥匙串条目失败，原因: {error}"),
            )),
        }
    }

    fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError> {
        self.entry(key)?.set_password(secret).map_err(|error| {
            AppError::with_detail(
                ErrorCode::SecretStoreUnavailable,
                format!("failed to write keychain entry: {error}"),
            )
        })
    }

    fn delete(&self, key: &SecretKey) -> Result<(), AppError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(AppError::with_detail(
                ErrorCode::SecretStoreUnavailable,
                format!("failed to delete keychain entry: {error}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::{ErrorCode, SecretKind, SessionId};

    #[test]
    fn keychain_round_trip_when_available() {
        let store = KeyringSecretStore::new("transom-test");
        let key = SecretKey::new(SessionId::new(), SecretKind::Password);
        match store.set(&key, "secret") {
            Ok(()) => {
                let loaded = store.get(&key).unwrap().unwrap();
                assert_eq!(loaded.as_str(), "secret");
                let _ = store.delete(&key);
            }
            Err(error) if error.code == ErrorCode::SecretStoreUnavailable => {
                // Headless Linux sessions often have no Secret Service.
            }
            Err(error) => panic!("{error}"),
        }
    }
}
