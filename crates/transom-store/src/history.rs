//! 命令历史的 JSON 持久化。
//!
//! 设计原理（Why）：
//! - 独立一份 `history.json` 而不是塞进 `sessions.json`：历史是"用过什么"，
//!   会话目录是"有哪些机器"，两者生命周期与体积都不同；混在一起会让每次
//!   敲一条命令都重写整份会话库，也会让会话备份带上与迁移无关的内容；
//! - 读写都走既有的原子写（临时文件 + rename，0600 权限）：历史里可能出现
//!   带 token 的命令行，权限与崩溃一致性都要按敏感文件对待；
//! - 读失败一律降级成空历史而不是报错：建议功能没了远比整个应用起不来轻。

use crate::file_io::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use transom_core::{AppError, HistoryStore};

const HISTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default = "history_version")]
    version: u32,
    #[serde(default)]
    commands: Vec<String>,
}

fn history_version() -> u32 {
    HISTORY_SCHEMA_VERSION
}

/// 命令历史文件读写。
pub struct JsonHistoryStore {
    path: PathBuf,
}

impl JsonHistoryStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// 读取历史；文件不存在、损坏或版本超前都返回空表并记一条 warn。
    pub fn load(&self) -> Vec<String> {
        match read_json::<HistoryFile>(&self.path) {
            Ok(Some(file)) => file.commands,
            Ok(None) => Vec::new(),
            Err(error) => {
                tracing::warn!(
                    "读取命令历史失败，按空历史启动，路径: {:?}，原因: {error}",
                    self.path
                );
                Vec::new()
            }
        }
    }

    /// 原子写入历史。
    ///
    /// # Errors
    /// 目录创建、序列化或改名失败时返回底层 IO 错误。
    pub fn save(&self, commands: &[String]) -> Result<(), AppError> {
        write_json(
            &self.path,
            &HistoryFile {
                version: HISTORY_SCHEMA_VERSION,
                commands: commands.to_vec(),
            },
        )
    }
}

impl HistoryStore for JsonHistoryStore {
    fn load(&self) -> Vec<String> {
        JsonHistoryStore::load(self)
    }

    fn save(&self, commands: &[String]) -> Result<(), AppError> {
        JsonHistoryStore::save(self, commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_and_missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let store = JsonHistoryStore::new(dir.path().join("history.json"));
        assert!(store.load().is_empty());

        store.save(&["git status".to_string()]).unwrap();
        assert_eq!(store.load(), vec!["git status".to_string()]);
        // 二次保存覆盖而非追加：去重与排序都由 core 的策略负责。
        store
            .save(&["ls -la".to_string(), "git status".to_string()])
            .unwrap();
        assert_eq!(store.load().len(), 2);
    }

    /// 坏文件不能让应用起不来：读侧必须降级成空历史。
    #[test]
    fn corrupt_file_degrades_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = JsonHistoryStore::new(&path);
        assert!(store.load().is_empty());
    }

    /// 保存后文件权限收紧到仅属主可读：历史里可能出现带 token 的命令行。
    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        let store = JsonHistoryStore::new(&path);
        store.save(&["echo token=abc".to_string()]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "实际权限: {mode:#o}");
    }
}
