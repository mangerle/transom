use crate::file_io::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use transom_core::{AppError, AppSettings, ErrorCode, SettingsStore};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    settings: AppSettings,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

pub struct JsonSettingsStore {
    path: PathBuf,
}

impl JsonSettingsStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Result<AppSettings, AppError> {
        let Some(file) = read_json::<SettingsFile>(&self.path)? else {
            return Ok(AppSettings::default());
        };
        if file.version > SCHEMA_VERSION {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                "settings file is newer than this application",
            ));
        }
        Ok(file.settings)
    }

    fn save(&self, settings: &AppSettings) -> Result<(), AppError> {
        write_json(
            &self.path,
            &SettingsFile {
                version: SCHEMA_VERSION,
                settings: settings.clone(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn settings_round_trip() {
        let dir = tempdir().unwrap();
        let store = JsonSettingsStore::new(dir.path().join("settings.json"));
        let mut settings = AppSettings {
            language: "en".into(),
            ..Default::default()
        };
        settings.terminal_interaction.copy_on_select = false;
        store.save(&settings).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.language, "en");
        assert!(!loaded.terminal_interaction.copy_on_select);
    }
}
