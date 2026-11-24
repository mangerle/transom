//! API 项目登记表的 JSON 持久化。
//!
//! 设计原理（Why）：API 项目与 Git 项目同属本机工具数据，使用同一条原子写入
//! 路径和坏文件隔离策略，避免 UI 层为了保存接口定义另造一套 I/O 语义。

use crate::file_io::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use transom_api::{API_SCHEMA_VERSION, ApiProject, ApiProjectStore, ApiRegistry};
use transom_core::{AppError, ErrorCode};

#[derive(Debug, Serialize, Deserialize)]
struct ProjectsFile {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    projects: Vec<transom_api::ApiProject>,
}

#[derive(Serialize)]
struct ProjectsFileRef<'a> {
    version: u32,
    projects: &'a [ApiProject],
}

fn schema_version() -> u32 {
    API_SCHEMA_VERSION
}

/// JSON 文件实现的 API 项目存储。
pub struct JsonApiProjectStore {
    path: PathBuf,
}

impl JsonApiProjectStore {
    /// 用指定文件路径构造存储。
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl ApiProjectStore for JsonApiProjectStore {
    fn load(&self) -> Result<ApiRegistry, AppError> {
        let Some(file) = read_json::<ProjectsFile>(&self.path)? else {
            return Ok(ApiRegistry::default());
        };
        if file.version > API_SCHEMA_VERSION {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "API 项目文件版本 {} 高于当前支持版本 {}，请升级 Transom",
                    file.version, API_SCHEMA_VERSION
                ),
            ));
        }
        Ok(ApiRegistry {
            version: API_SCHEMA_VERSION,
            projects: file.projects,
        })
    }

    fn save(&self, registry: &ApiRegistry) -> Result<(), AppError> {
        write_json(
            &self.path,
            &ProjectsFileRef {
                version: API_SCHEMA_VERSION,
                projects: &registry.projects,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_loads_empty_registry() {
        let dir = tempdir().expect("临时目录");
        let store = JsonApiProjectStore::new(dir.path().join("api_projects.json"));
        assert!(store.load().expect("缺文件为空").projects.is_empty());
    }

    #[test]
    fn legacy_mock_routes_are_ignored_and_removed_on_save() {
        let dir = tempdir().expect("临时目录");
        let path = dir.path().join("api_projects.json");
        let mut project =
            serde_json::to_value(transom_api::ApiProject::new("旧项目")).expect("项目应可序列化");
        project.as_object_mut().expect("项目应为 JSON 对象").insert(
            "mock_routes".to_string(),
            serde_json::json!([{"method": "GET", "path": "/ping"}]),
        );
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": API_SCHEMA_VERSION,
                "projects": [project],
            }))
            .expect("登记表应可序列化"),
        )
        .expect("写旧项目文件");

        let store = JsonApiProjectStore::new(&path);
        let loaded = store.load().expect("旧项目应可读取");
        assert_eq!(loaded.projects[0].name, "旧项目");
        store.save(&loaded).expect("重新保存成功");
        let saved = std::fs::read_to_string(&path).expect("读取新项目文件");
        assert!(
            !saved.contains("mock_routes"),
            "新文件不应继续保存 Mock 路由"
        );
    }

    #[test]
    fn round_trips_projects_and_requests() {
        let dir = tempdir().expect("临时目录");
        let store = JsonApiProjectStore::new(dir.path().join("api_projects.json"));
        let mut project = transom_api::ApiProject::new("演示 API");
        let request = transom_api::ApiRequest::new("健康检查", None);
        project.upsert_request(request.clone());
        let registry = ApiRegistry {
            version: API_SCHEMA_VERSION,
            projects: vec![project.clone()],
        };
        store.save(&registry).expect("保存成功");
        let loaded = store.load().expect("读取成功");
        assert_eq!(loaded.projects, vec![project]);
    }
}
