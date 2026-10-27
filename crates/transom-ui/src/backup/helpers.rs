//! 备份文件格式与说明辅助工具。

use crate::palette::localized_error;
use std::path::PathBuf;
use transom_core::{AppError, SecretRestoreReport};

/// 口令写回结果的附加说明。
pub(crate) fn secret_restore_note(secrets: Result<SecretRestoreReport, AppError>) -> String {
    match secrets {
        Ok(report) if report.restored > 0 || report.kept > 0 => format!(
            " {}",
            rust_i18n::t!(
                "backup.imported_secrets",
                restored = report.restored,
                kept = report.kept
            )
        ),
        Ok(_) => String::new(),
        Err(error) => format!(
            " {}: {}",
            rust_i18n::t!("backup.imported_secrets_failed"),
            localized_error(&error)
        ),
    }
}

pub(super) fn ensure_backup_extension(path: PathBuf) -> PathBuf {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("json" | "yaml" | "yml") => path,
        _ => path.with_extension("json"),
    }
}
