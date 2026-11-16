//! 对比面板的本地文件读写：底层 IO 下沉至 `transom-store`，本模块仅做国际化文案映射。

use std::path::Path;
use transom_store::{LocalTextError, read_utf8_limited, write_utf8_checked_replace};

/// 读取本地 UTF-8 文本文件。
pub(crate) fn read_local_text(path: &Path) -> Result<String, String> {
    read_utf8_limited(path, super::MAX_DIFF_BYTES).map_err(|error| localize_error(path, error))
}

/// 原子替换本地文本文件，保留原有权限位。
pub(crate) fn write_local_text(path: &Path, expected: &str, text: &str) -> Result<(), String> {
    write_utf8_checked_replace(path, expected, text, super::MAX_DIFF_BYTES)
        .map_err(|error| localize_error(path, error))
}

fn localize_error(path: &Path, error: LocalTextError) -> String {
    match error {
        LocalTextError::Metadata(error) => {
            rust_i18n::t!("diff.error_metadata", path = path.display(), error = error).to_string()
        }
        LocalTextError::NotAFile => {
            rust_i18n::t!("diff.error_not_file", path = path.display()).to_string()
        }
        LocalTextError::TooLarge { limit } => {
            rust_i18n::t!("diff.error_too_large", limit = limit, path = path.display()).to_string()
        }
        LocalTextError::TooLargeDuringRead => {
            rust_i18n::t!("diff.error_too_large_during_read", path = path.display()).to_string()
        }
        LocalTextError::Read(error) => {
            rust_i18n::t!("diff.error_read", path = path.display(), error = error).to_string()
        }
        LocalTextError::NotUtf8 => {
            rust_i18n::t!("diff.error_not_utf8", path = path.display()).to_string()
        }
        LocalTextError::Modified => {
            rust_i18n::t!("diff.error_modified", path = path.display()).to_string()
        }
        LocalTextError::NoParent => {
            rust_i18n::t!("diff.error_no_parent", path = path.display()).to_string()
        }
        LocalTextError::BadPath => {
            rust_i18n::t!("diff.error_bad_path", path = path.display()).to_string()
        }
        LocalTextError::TempWrite { path, error } => rust_i18n::t!(
            "diff.error_temp_write",
            path = path.display(),
            error = error
        )
        .to_string(),
        LocalTextError::SetPermissions { path, error } => rust_i18n::t!(
            "diff.error_set_permissions",
            path = path.display(),
            error = error
        )
        .to_string(),
        LocalTextError::Replace(error) => {
            rust_i18n::t!("diff.error_replace", path = path.display(), error = error).to_string()
        }
    }
}
