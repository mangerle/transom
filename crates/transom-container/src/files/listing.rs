//! 目录 tar 解析、子项聚合与容器内路径拼装。
use super::MAX_LIST_ENTRIES;
use crate::error::ContainerError;
use std::collections::BTreeMap;
use std::io::Read;
use transom_core::{ContainerFileEntry, ContainerFileKind};

fn join_container(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// 把 tar 条目路径换算为相对请求路径的路径；不相关条目返回 `None`。
pub(super) fn relativize(raw: &str, base_name: &str) -> Option<String> {
    let trimmed = raw.trim_start_matches('/').trim_start_matches("./");
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return None;
    }
    if base_name.is_empty() {
        return Some(trimmed.to_string());
    }
    if trimmed == base_name {
        return None;
    }
    let prefix = format!("{base_name}/");
    trimmed
        .strip_prefix(&prefix)
        .map(str::to_string)
        .or_else(|| Some(trimmed.to_string()))
}

pub(super) fn parse_listing<R: Read>(
    reader: R,
    base: &str,
) -> Result<Vec<ContainerFileEntry>, ContainerError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| ContainerError::File {
        path: base.to_string(),
        message: format!("读取 tar 失败: {error}"),
    })?;
    let base_name = base.rsplit('/').next().unwrap_or("");
    let mut entries_map: BTreeMap<String, ContainerFileEntry> = BTreeMap::new();
    let mut count = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| ContainerError::File {
            path: base.to_string(),
            message: format!("解析 tar 条目失败: {error}"),
        })?;
        count += 1;
        if count > MAX_LIST_ENTRIES {
            return Err(ContainerError::File {
                path: base.to_string(),
                message: format!("目录条目超过上限 {MAX_LIST_ENTRIES}"),
            });
        }
        let raw = entry.path().map_err(|error| ContainerError::File {
            path: base.to_string(),
            message: format!("解析 tar 路径失败: {error}"),
        })?;
        let raw = raw.to_string_lossy().to_string();
        let Some(relative) = relativize(&raw, base_name) else {
            continue;
        };
        let mut components = relative.split('/').filter(|part| !part.is_empty());
        let Some(first) = components.next() else {
            continue;
        };
        let direct = components.next().is_none();
        let path = join_container(base, first);
        if !direct {
            entries_map
                .entry(first.to_string())
                .or_insert(ContainerFileEntry {
                    name: first.to_string(),
                    path,
                    kind: ContainerFileKind::Directory,
                    size: 0,
                    mode: 0,
                    modified: None,
                    link_target: None,
                });
            continue;
        }
        let header = entry.header();
        let entry_type = header.entry_type();
        let kind = if entry_type.is_dir() {
            ContainerFileKind::Directory
        } else if entry_type.is_symlink() {
            ContainerFileKind::Symlink
        } else if entry_type.is_file() {
            ContainerFileKind::File
        } else {
            ContainerFileKind::Other
        };
        let link_target = if kind == ContainerFileKind::Symlink {
            entry
                .link_name()
                .ok()
                .flatten()
                .map(|path| path.to_string_lossy().to_string())
        } else {
            None
        };
        entries_map.insert(
            first.to_string(),
            ContainerFileEntry {
                name: first.to_string(),
                path,
                kind,
                size: header.size().unwrap_or(0),
                mode: header.mode().unwrap_or(0),
                modified: header.mtime().ok().map(|time| time as i64),
                link_target,
            },
        );
    }
    let mut list: Vec<ContainerFileEntry> = entries_map.into_values().collect();
    list.sort_by(|left, right| {
        let left_dir = left.kind == ContainerFileKind::Directory;
        let right_dir = right.kind == ContainerFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(list)
}

pub(super) fn read_first_file<R: Read>(
    reader: R,
    path: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool, u64), ContainerError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| ContainerError::File {
        path: path.to_string(),
        message: format!("读取 tar 失败: {error}"),
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| ContainerError::File {
            path: path.to_string(),
            message: format!("解析 tar 条目失败: {error}"),
        })?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let size = entry.header().size().unwrap_or(0);
        let take = size.min(max_bytes as u64) as usize;
        let mut data = vec![0u8; take];
        entry
            .read_exact(&mut data)
            .map_err(|error| ContainerError::File {
                path: path.to_string(),
                message: format!("读取文件内容失败: {error}"),
            })?;
        return Ok((data, size > max_bytes as u64, size));
    }
    Err(ContainerError::NotFound {
        target: path.to_string(),
    })
}
