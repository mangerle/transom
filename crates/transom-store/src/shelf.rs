//! Git 离线搁置项（Shelf）的 JSON 持久化。
//!
//! 设计原理（Why）：
//! - 搁置项（Shelf）是 IDE 维护的独立离线补丁集，完全独立于 Git 内部的 stash 栈；
//! - 针对每个项目维护一份独立的 JSON 文件，以项目 ID 命名；
//! - 结构体包含补丁元信息（分支、创建时间、文件清单）及 unified patch 补丁文本；
//! - 使用原子写保证断电与崩溃时数据不损坏。

use crate::file_io::{read_json, write_json};
use crate::paths::shelf_dir;
use std::fs;
use transom_core::{AppError, ShelfEntry};

/// 搁置项文件版本。
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ShelfStoreFile {
    #[serde(default = "schema_version")]
    version: u32,
    #[serde(default)]
    shelves: Vec<ShelfEntry>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

/// 读取指定项目的全部离线搁置项。
pub fn load_project_shelves(project_id: &str) -> Result<Vec<ShelfEntry>, AppError> {
    let dir = shelf_dir();
    let path = dir.join(format!("{project_id}.json"));
    let Some(file) = read_json::<ShelfStoreFile>(&path)? else {
        return Ok(Vec::new());
    };
    Ok(file.shelves)
}

/// 保存指定项目的全部离线搁置项。
pub fn save_project_shelves(project_id: &str, shelves: &[ShelfEntry]) -> Result<(), AppError> {
    let dir = shelf_dir();
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    let path = dir.join(format!("{project_id}.json"));
    let file = ShelfStoreFile {
        version: SCHEMA_VERSION,
        shelves: shelves.to_vec(),
    };
    write_json(&path, &file)
}
