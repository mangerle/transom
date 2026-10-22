use crate::file_io::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use transom_core::{AppError, ErrorCode, Session, SessionStore};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SessionFile {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    sessions: Vec<Session>,
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
}

impl SessionStore for JsonSessionStore {
    fn load(&self) -> Result<Vec<Session>, AppError> {
        let Some(file) = read_json::<SessionFile>(&self.path)? else {
            return Ok(Vec::new());
        };
        if file.version > SCHEMA_VERSION {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "session file version {} is newer than supported version {SCHEMA_VERSION}",
                    file.version
                ),
            ));
        }
        Ok(file.sessions)
    }

    fn save(&self, sessions: &[Session]) -> Result<(), AppError> {
        write_json(
            &self.path,
            &SessionFile {
                version: SCHEMA_VERSION,
                sessions: sessions.to_vec(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use transom_core::{AuthMethod, Connection, SshConnection};

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
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn sessions_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let store = JsonSessionStore::new(&path);
        let sessions = vec![session("one"), session("two")];
        store.save(&sessions).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "one");
        assert_eq!(loaded[1].name, "two");
        assert!(path.exists());
    }

    #[test]
    fn newer_schema_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(&path, r#"{"version":99,"sessions":[]}"#).unwrap();
        let store = JsonSessionStore::new(&path);
        assert!(store.load().is_err());
    }
}
