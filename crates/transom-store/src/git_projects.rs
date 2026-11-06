//! Git 项目列表的 JSON 持久化。
//!
//! 设计原理（Why）：结构与 `settings.json` 同构（`version` + 载荷），读失败
//! 时按 `file_io` 的既有约定隔离坏文件并返回错误，而不是静默给一张空表——
//! 列表被清空对用户等同于数据丢失，必须让他看见。

use crate::file_io::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use transom_core::{AppError, GitFolder, GitProject, GitProjectStore, GitRegistry};

/// 当前支持的落盘格式版本：v2 起带分组列表。
const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct ProjectsFile {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    projects: Vec<GitProject>,
    /// 分组是 v2 才加的载荷。v1 文件没有这个键，按空分组读；项目里的
    /// `folder_id` 同样缺省为 `None`——旧文件不迁移也能直接打开，
    /// 不会出现「升级后列表被清空」这种等同数据丢失的表现。
    #[serde(default)]
    folders: Vec<GitFolder>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

pub struct JsonGitProjectStore {
    path: PathBuf,
}

impl JsonGitProjectStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl GitProjectStore for JsonGitProjectStore {
    fn load(&self) -> Result<GitRegistry, AppError> {
        let Some(file) = read_json::<ProjectsFile>(&self.path)? else {
            return Ok(GitRegistry::default());
        };
        if file.version > SCHEMA_VERSION {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!(
                    "Git 项目文件版本 {} 高于当前支持版本 {}，请升级 Transom",
                    file.version, SCHEMA_VERSION
                ),
            ));
        }
        Ok(GitRegistry {
            projects: file.projects,
            folders: file.folders,
        })
    }

    fn save(&self, registry: &GitRegistry) -> Result<(), AppError> {
        write_json(
            &self.path,
            &ProjectsFile {
                version: SCHEMA_VERSION,
                projects: registry.projects.clone(),
                folders: registry.folders.clone(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn missing_file_loads_empty_list() {
        let dir = tempdir().expect("临时目录");
        let store = JsonGitProjectStore::new(dir.path().join("git_projects.json"));
        let registry = store.load().expect("缺文件应按空表处理");
        assert!(registry.projects.is_empty());
        assert!(registry.folders.is_empty());
    }

    #[test]
    fn round_trips_projects() {
        let dir = tempdir().expect("临时目录");
        let store = JsonGitProjectStore::new(dir.path().join("git_projects.json"));
        let mut first = GitProject::new(PathBuf::from("/srv/repo-a"));
        first.label = Some("仓库 A".into());
        let second = GitProject::new(PathBuf::from("/srv/repo-b"));
        let registry = GitRegistry {
            projects: vec![first.clone(), second],
            folders: Vec::new(),
        };
        store.save(&registry).expect("保存成功");
        let loaded = store.load().expect("读取成功");
        assert_eq!(loaded.projects.len(), 2);
        assert_eq!(loaded.projects[0], first);
        assert_eq!(loaded.projects[0].display_name(), "仓库 A");
        assert_eq!(loaded.projects[1].display_name(), "repo-b");
    }

    /// 分组与项目必须一起回得来：分开写就会出现项目指着已消失分组的撕裂状态。
    #[test]
    fn round_trips_folders_and_appearance() {
        let dir = tempdir().expect("临时目录");
        let store = JsonGitProjectStore::new(dir.path().join("git_projects.json"));
        let folder = GitFolder::new("公司");
        let mut folded = GitProject::new(PathBuf::from("/srv/repo-a"));
        folded.folder_id = Some(folder.id);
        folded.icon = transom_core::SessionIcon::Database;
        folded.color = transom_core::SessionColor::Red;
        let outside = GitProject::new(PathBuf::from("/srv/repo-b"));
        store
            .save(&GitRegistry {
                projects: vec![folded.clone(), outside],
                folders: vec![folder.clone()],
            })
            .expect("保存成功");

        let loaded = store.load().expect("读取成功");
        assert_eq!(loaded.folders, vec![folder.clone()]);
        assert_eq!(loaded.projects[0], folded);
        assert_eq!(loaded.projects[0].folder_id, Some(folder.id));
        assert_eq!(loaded.projects[0].icon, transom_core::SessionIcon::Database);
        assert_eq!(loaded.projects[0].color, transom_core::SessionColor::Red);
        assert_eq!(loaded.projects[1].folder_id, None);
    }

    /// v1 老文件没有 `folders` 键，项目记录里也没有 `folder_id`/`icon`/`color`：
    /// 必须按「无分组 + 默认外观」读出来，而不是报错或清空列表。
    #[test]
    fn legacy_v1_file_loads_without_folders() {
        let dir = tempdir().expect("临时目录");
        let path = dir.path().join("git_projects.json");
        std::fs::write(
            &path,
            r#"{"version":1,"projects":[{"id":"6a1d1e2f-3b4c-5d6e-7f80-91a2b3c4d5e6","path":"/srv/legacy"}]}"#,
        )
        .expect("写入 v1 样本");

        let loaded = JsonGitProjectStore::new(&path)
            .load()
            .expect("v1 文件应能直接打开");
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.folders.len(), 0);
        assert_eq!(loaded.projects[0].folder_id, None);
        assert_eq!(loaded.projects[0].display_name(), "legacy");
    }

    #[test]
    fn newer_schema_version_is_rejected() {
        let dir = tempdir().expect("临时目录");
        let path = dir.path().join("git_projects.json");
        std::fs::write(&path, r#"{"version": 99, "projects": [], "folders": []}"#)
            .expect("写入样本");
        let store = JsonGitProjectStore::new(&path);
        let error = store.load().expect_err("高版本文件必须拒绝而不是清空");
        assert_eq!(error.code, transom_core::ErrorCode::Configuration);
    }
}
