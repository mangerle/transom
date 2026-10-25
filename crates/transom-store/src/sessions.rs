//! 会话与文件夹的 JSON 持久化。
//!
//! 版本 2 引入显式文件夹树；版本 1 的 `session.group` 扁平字符串在加载时
//! 迁移为多级文件夹。迁移只发生一次，写回后即固定为版本 2。

use crate::file_io::{read_json, write_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use transom_core::{
    AppError, ErrorCode, Identity, Session, SessionCatalog, SessionFolder, SessionStore,
};

const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize)]
struct SessionFileRef<'a> {
    version: u32,
    folders: &'a [SessionFolder],
    sessions: &'a [Session],
    /// 凭据模板与会话同文件存放：会话引用模板时，两者必须一起走才有意义，
    /// 拆成两个文件会多出一份"只同步了一半"的错配可能。
    identities: &'a [Identity],
}

#[derive(Debug, Deserialize)]
struct SessionFileRaw {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    folders: Vec<SessionFolder>,
    #[serde(default)]
    sessions: Vec<Value>,
    #[serde(default)]
    identities: Vec<Identity>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

pub struct JsonSessionStore {
    path: PathBuf,
}

impl JsonSessionStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    fn decode(&self, file: SessionFileRaw) -> Result<SessionCatalog, AppError> {
        let mut catalog = SessionCatalog {
            folders: file.folders,
            sessions: Vec::with_capacity(file.sessions.len()),
            identities: file.identities,
        };
        catalog.normalize();
        for value in file.sessions {
            let legacy_group = value
                .get("group")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let mut session: Session = serde_json::from_value(value).map_err(|error| {
                AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "解析会话记录失败，文件: {}，原因: {error}",
                        self.path.display()
                    ),
                )
            })?;
            if session.folder_id.is_none()
                && let Some(group) = legacy_group
            {
                session.folder_id = catalog.ensure_folder_path(&group);
            }
            catalog.sessions.push(session);
        }
        catalog.normalize();
        Ok(catalog)
    }
}

impl SessionStore for JsonSessionStore {
    fn load(&self) -> Result<SessionCatalog, AppError> {
        let Some(file) = read_json::<SessionFileRaw>(&self.path)? else {
            return Ok(SessionCatalog::default());
        };
        if file.version > SCHEMA_VERSION {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "会话文件版本 {} 高于当前支持版本 {}，请升级 Transom",
                    file.version, SCHEMA_VERSION
                ),
            ));
        }
        self.decode(file)
    }

    fn save(&self, catalog: &SessionCatalog) -> Result<(), AppError> {
        write_json(
            &self.path,
            &SessionFileRef {
                version: SCHEMA_VERSION,
                folders: &catalog.folders,
                sessions: &catalog.sessions,
                identities: &catalog.identities,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use transom_core::{AuthMethod, Connection, FolderId, SshConnection};

    fn session(name: &str) -> Session {
        Session::new(
            name,
            Connection::Ssh(SshConnection {
                host: "example.com".into(),
                port: 22,
                username: "user".into(),
                auth: AuthMethod::Password { save: true },
                options: Default::default(),
            }),
        )
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = tempdir().unwrap();
        let store = JsonSessionStore::new(dir.path().join("sessions.json"));
        let catalog = store.load().unwrap();
        assert!(catalog.sessions.is_empty());
        assert!(catalog.folders.is_empty());
    }

    #[test]
    fn catalog_round_trips_with_nested_folders() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let store = JsonSessionStore::new(&path);

        let mut catalog = SessionCatalog::default();
        let leaf = catalog.ensure_folder_path("生产环境/数据库").unwrap();
        let mut prod = session("db-01");
        prod.folder_id = Some(leaf);
        catalog.sessions.push(prod);
        catalog.sessions.push(session("standalone"));
        store.save(&catalog).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.folders.len(), 2);
        assert_eq!(loaded.sessions[0].folder_id, Some(leaf));
        assert_eq!(
            loaded.folder_path(leaf).as_deref(),
            Some("生产环境 / 数据库")
        );
        assert_eq!(loaded.sessions[1].folder_id, None);
    }

    #[test]
    fn version_one_group_is_migrated_to_nested_folders() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let mut legacy_session = serde_json::to_value(session("db")).unwrap();
        legacy_session
            .as_object_mut()
            .unwrap()
            .insert("group".into(), serde_json::json!("生产/数据库"));
        let legacy = serde_json::json!({
            "version": 1,
            "sessions": [legacy_session]
        });
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let catalog = JsonSessionStore::new(&path).load().unwrap();
        assert_eq!(catalog.sessions.len(), 1);
        assert_eq!(catalog.folders.len(), 2);
        let folder_id = catalog.sessions[0].folder_id.unwrap();
        assert_eq!(
            catalog.folder_path(folder_id).as_deref(),
            Some("生产 / 数据库")
        );

        // 旧版 group 字符串不应再出现在写回文件中。
        let store = JsonSessionStore::new(&path);
        store.save(&catalog).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"group\""));
        assert!(text.contains("\"version\": 2"));
    }

    #[test]
    fn newer_schema_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(&path, r#"{"version":99,"sessions":[]}"#).unwrap();
        let store = JsonSessionStore::new(&path);
        assert!(store.load().is_err());
    }

    #[test]
    fn broken_folder_links_are_repaired_on_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let mut orphan = serde_json::to_value(session("x")).unwrap();
        orphan.as_object_mut().unwrap().insert(
            "folder_id".into(),
            serde_json::json!(FolderId::new().to_string()),
        );
        let content = serde_json::json!({
            "version": 2,
            "folders": [],
            "sessions": [orphan]
        });
        std::fs::write(&path, serde_json::to_vec(&content).unwrap()).unwrap();
        let catalog = JsonSessionStore::new(&path).load().unwrap();
        assert_eq!(catalog.sessions[0].folder_id, None);
    }
}
