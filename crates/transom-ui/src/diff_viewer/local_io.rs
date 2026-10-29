//! 对比面板的本地文件读写：阻塞实现，统一由后台线程调度。
//!
//! 设计原理（Why）：
//! - 读：先看 `metadata().len()` 再读，超限直接拒绝，避免把超大文件整份拉进内存；
//!   读后仍按 UTF-8 校验，二进制文件在差异视图里没有意义；
//! - 写：同目录临时文件 + 重命名，保证写到一半崩溃不会留下半截文件；
//!   先复制原文件权限位，避免合并后脚本失去执行位。
//!
//! 代价与局限：
//! - 不生成本地备份文件：历史版本交给用户自己的版本控制；
//! - Windows 上重命名覆盖已存在文件会失败，失败时回退为"删除后重命名"，
//!   该回退不是原子操作（本项目主要目标平台为 Linux）。

use std::path::Path;

/// 读取本地 UTF-8 文本文件。
pub(crate) fn read_local_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|error| {
        rust_i18n::t!("diff.error_metadata", path = path.display(), error = error).to_string()
    })?;
    if !meta.is_file() {
        return Err(rust_i18n::t!("diff.error_not_file", path = path.display()).to_string());
    }
    if meta.len() > super::MAX_DIFF_BYTES {
        return Err(rust_i18n::t!(
            "diff.error_too_large",
            limit = super::MAX_DIFF_BYTES,
            path = path.display()
        )
        .to_string());
    }
    let bytes = std::fs::read(path).map_err(|error| {
        rust_i18n::t!("diff.error_read", path = path.display(), error = error).to_string()
    })?;
    if bytes.len() as u64 > super::MAX_DIFF_BYTES {
        // 元数据与真实读取之间文件可能被追加；以真实字节数为准再拦一次。
        return Err(
            rust_i18n::t!("diff.error_too_large_during_read", path = path.display()).to_string(),
        );
    }
    String::from_utf8(bytes)
        .map_err(|_| rust_i18n::t!("diff.error_not_utf8", path = path.display()).to_string())
}

/// 原子替换本地文本文件，保留原有权限位。
///
/// 写前重新读取并与 `expected` 比对：对比面板可能开了很久，期间用户或别的程序
/// 改过文件；不比对就覆盖等于静默丢弃别人的修改。比对与替换之间仍有极小的
/// TOCTOU 窗口，但已把"长时间滞留导致的覆盖"这类真实场景挡住。
pub(crate) fn write_local_text(path: &Path, expected: &str, text: &str) -> Result<(), String> {
    let current = read_local_text(path)?;
    if current != expected {
        return Err(rust_i18n::t!("diff.error_modified", path = path.display()).to_string());
    }
    let meta = std::fs::metadata(path).map_err(|error| {
        rust_i18n::t!("diff.error_metadata", path = path.display(), error = error).to_string()
    })?;
    let parent = path
        .parent()
        .ok_or_else(|| rust_i18n::t!("diff.error_no_parent", path = path.display()).to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| rust_i18n::t!("diff.error_bad_path", path = path.display()).to_string())?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.transom-tmp-{}", std::process::id()));
    let result = (|| {
        std::fs::write(&temp, text).map_err(|error| {
            rust_i18n::t!(
                "diff.error_temp_write",
                path = temp.display(),
                error = error
            )
            .to_string()
        })?;
        std::fs::set_permissions(&temp, meta.permissions()).map_err(|error| {
            rust_i18n::t!(
                "diff.error_set_permissions",
                path = temp.display(),
                error = error
            )
            .to_string()
        })?;
        match std::fs::rename(&temp, path) {
            Ok(()) => Ok(()),
            Err(_) if cfg!(windows) => {
                // Windows 不允许 rename 覆盖既有文件；回退路径牺牲原子性换取可用性。
                std::fs::remove_file(path).map_err(|error| {
                    rust_i18n::t!("diff.error_replace", path = path.display(), error = error)
                        .to_string()
                })?;
                std::fs::rename(&temp, path).map_err(|error| {
                    rust_i18n::t!("diff.error_replace", path = path.display(), error = error)
                        .to_string()
                })
            }
            Err(error) => {
                Err(
                    rust_i18n::t!("diff.error_replace", path = path.display(), error = error)
                        .to_string(),
                )
            }
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
