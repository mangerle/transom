//! 启动期的镜像仓库密码迁移与钥匙串回填。
//!
//! 设计原理（Why）：历史版本把仓库密码明文写在 `settings.json` 里；
//! 现在设置文件只保留元数据，密码统一托管到系统钥匙串。启动时先把旧明文
//! 迁移到钥匙串（成功后仍需保留在内存中供本次会话使用），再从钥匙串回填
//! 没有明文的仓库，最后让调用方把清理后的设置重新落盘。

use transom_core::{AppSettings, SecretKey, SecretStore};

/// 把设置中的仓库密码迁移到钥匙串，并回填本次会话所需的密码。
///
/// 返回 `true` 表示设置内容发生了需要落盘的变化（存在被迁移的旧明文密码）。
/// 迁移失败只记录告警并保留内存中的密码，不阻断应用启动。
pub(crate) fn hydrate_registry_passwords(
    settings: &mut AppSettings,
    store: &dyn SecretStore,
) -> bool {
    let mut settings_changed = false;
    for registry in &mut settings.container_registries {
        let key = SecretKey::registry_password(&registry.id);
        if !registry.password.is_empty() {
            match store.set(&key, &registry.password) {
                Ok(()) => {
                    // 不能清空内存值：本次会话的搜索/拉取仍要使用它。
                    // 落盘时 `ContainerRegistry::password` 会被跳过序列化。
                    settings_changed = true;
                    log::info!("镜像仓库密码已迁移到系统钥匙串，仓库: {}", registry.id);
                }
                Err(error) => {
                    log::warn!(
                        "镜像仓库密码迁移到系统钥匙串失败，仓库: {}，原因: {error}",
                        registry.id
                    );
                }
            }
            continue;
        }
        match store.get(&key) {
            Ok(Some(secret)) => registry.password = secret.to_string(),
            Ok(None) => {}
            Err(error) => {
                log::warn!(
                    "从系统钥匙串读取镜像仓库密码失败，仓库: {}，原因: {error}",
                    registry.id
                );
            }
        }
    }
    settings_changed
}

#[cfg(test)]
mod tests {
    use super::hydrate_registry_passwords;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use transom_core::{
        AppError, AppSettings, ContainerRegistry, ErrorCode, SecretKey, SecretStore,
    };

    #[derive(Default)]
    struct MemorySecretStore {
        values: Mutex<HashMap<String, String>>,
    }

    impl SecretStore for MemorySecretStore {
        fn get(&self, key: &SecretKey) -> Result<Option<zeroize::Zeroizing<String>>, AppError> {
            Ok(self
                .values
                .lock()
                .expect("测试锁不可中毒")
                .get(&key.account())
                .cloned()
                .map(zeroize::Zeroizing::new))
        }

        fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError> {
            self.values
                .lock()
                .expect("测试锁不可中毒")
                .insert(key.account(), secret.to_string());
            Ok(())
        }

        fn delete(&self, key: &SecretKey) -> Result<(), AppError> {
            self.values
                .lock()
                .expect("测试锁不可中毒")
                .remove(&key.account());
            Ok(())
        }
    }

    #[test]
    fn legacy_plaintext_password_moves_to_keyring_and_stays_in_memory() {
        let store = MemorySecretStore::default();
        let mut settings = AppSettings::default();
        settings.container_registries.push(ContainerRegistry {
            id: "harbor".into(),
            name: "Harbor".into(),
            url: "https://harbor.example.com".into(),
            username: "user".into(),
            password: "legacy-secret".into(),
            is_default: false,
        });

        assert!(hydrate_registry_passwords(&mut settings, &store));
        let registry = settings
            .container_registries
            .iter()
            .find(|registry| registry.id == "harbor")
            .expect("测试仓库必须存在");
        assert_eq!(registry.password, "legacy-secret");
        let stored = store
            .get(&SecretKey::registry_password("harbor"))
            .expect("读取不得失败")
            .expect("必须已写入");
        assert_eq!(stored.as_str(), "legacy-secret");
    }

    #[test]
    fn empty_password_is_hydrated_from_keyring() {
        let store = MemorySecretStore::default();
        store
            .set(&SecretKey::registry_password("harbor"), "from-keyring")
            .expect("写入不得失败");
        let mut settings = AppSettings::default();
        settings.container_registries.push(ContainerRegistry {
            id: "harbor".into(),
            name: "Harbor".into(),
            url: "https://harbor.example.com".into(),
            username: "user".into(),
            password: String::new(),
            is_default: false,
        });

        assert!(!hydrate_registry_passwords(&mut settings, &store));
        let registry = settings
            .container_registries
            .iter()
            .find(|registry| registry.id == "harbor")
            .expect("测试仓库必须存在");
        assert_eq!(registry.password, "from-keyring");
    }

    #[test]
    fn settings_json_never_serializes_registry_password() {
        let registry = ContainerRegistry {
            id: "harbor".into(),
            name: "Harbor".into(),
            url: "https://harbor.example.com".into(),
            username: "user".into(),
            password: "must-not-leak".into(),
            is_default: false,
        };
        let json = serde_json::to_string(&registry).expect("序列化不得失败");
        assert!(!json.contains("must-not-leak"));
        assert!(!json.contains("password"));
    }

    #[test]
    fn unavailable_keyring_keeps_legacy_password_for_current_session() {
        struct FailingStore;
        impl SecretStore for FailingStore {
            fn get(
                &self,
                _key: &SecretKey,
            ) -> Result<Option<zeroize::Zeroizing<String>>, AppError> {
                Err(AppError::with_detail(
                    ErrorCode::SecretStoreUnavailable,
                    "钥匙串不可用",
                ))
            }

            fn set(&self, _key: &SecretKey, _secret: &str) -> Result<(), AppError> {
                Err(AppError::with_detail(
                    ErrorCode::SecretStoreUnavailable,
                    "钥匙串不可用",
                ))
            }

            fn delete(&self, _key: &SecretKey) -> Result<(), AppError> {
                Err(AppError::with_detail(
                    ErrorCode::SecretStoreUnavailable,
                    "钥匙串不可用",
                ))
            }
        }

        let mut settings = AppSettings::default();
        settings.container_registries.push(ContainerRegistry {
            id: "harbor".into(),
            name: "Harbor".into(),
            url: "https://harbor.example.com".into(),
            username: "user".into(),
            password: "still-here".into(),
            is_default: false,
        });
        assert!(!hydrate_registry_passwords(&mut settings, &FailingStore));
        let registry = settings
            .container_registries
            .iter()
            .find(|registry| registry.id == "harbor")
            .expect("测试仓库必须存在");
        assert_eq!(registry.password, "still-here");
    }
}
