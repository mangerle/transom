//! 本地目录列举：目录优先、名称不区分大小写排序。
//!
//! 设计原理（Why）：本地栏、本地终端文件面板与拖拽上传共用同一份扫描逻辑，
//! 排序规则（目录在前、忽略大小写）在这里单点维护，避免各调用点各排一套。
//!
//! 代价与局限：`std::fs` 是阻塞调用，高层必须经后台执行器调度，不得在 UI
//! 线程直接调用。

use super::LocalFileEntry;
use std::path::Path;
use transom_core::{AppError, ErrorCode};

/// 本地文件系统扫描器。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LocalVfsProvider;

impl LocalVfsProvider {
    /// 同步扫描本地目录并返回排序后的条目列表。
    ///
    /// # Errors
    /// 目录不可读时返回 `Io` 错误，信息附带底层原因。
    pub(crate) fn list_sync(dir: &Path) -> Result<Vec<LocalFileEntry>, AppError> {
        let read_dir = std::fs::read_dir(dir).map_err(|error| {
            AppError::with_detail(ErrorCode::Io, format!("读取本地目录失败: {error}"))
        })?;
        let mut entries = Vec::new();
        for item in read_dir.flatten() {
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_string();
            let is_dir = item.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            // 符号链接按普通条目列出（与既有行为一致），大小取目标元数据长度。
            let size = if is_dir {
                0
            } else {
                item.metadata().map(|meta| meta.len()).unwrap_or(0)
            };
            entries.push(LocalFileEntry {
                name,
                path,
                is_dir,
                size,
            });
        }

        // 统一排序：目录优先，随后按名称小写字母序排序
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
        });
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::LocalVfsProvider;
    use std::path::Path;

    #[test]
    fn local_provider_lists_current_dir_with_dirs_first() {
        let entries = LocalVfsProvider::list_sync(Path::new(".")).unwrap();
        assert!(!entries.is_empty());
        let first_file = entries.iter().position(|entry| !entry.is_dir);
        let last_dir = entries.iter().rposition(|entry| entry.is_dir);
        if let (Some(file), Some(dir)) = (first_file, last_dir) {
            assert!(dir < file, "目录应当排列在普通文件前面");
        }
    }
}
