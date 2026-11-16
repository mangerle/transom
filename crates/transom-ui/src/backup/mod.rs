//! 会话目录备份的导入导出交互。

//!
//! 加密与否由用户在弹窗里勾选：加密备份用 Argon2id 派生密钥并以
//! XChaCha20-Poly1305 认证加密写入信封；明文备份直接落一份 JSON/YAML，
//! 便于人工查看与迁移，但文件里的主机名、用户名是明文。
//!
//! 弹窗状态、后台任务与渲染归 [`BackupPromptView`] 实体；本模块保留跨模块共享的数据契约与轻量工具。

use crate::palette::localized_error;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use std::path::PathBuf;
use transom_core::{AppError, SecretRestoreReport};

/// 备份导出/导入动作及其目标路径。
#[derive(Debug, Clone)]
pub(crate) enum BackupAction {
    Export,
    Import(PathBuf),
}

impl BackupAction {
    pub(crate) fn is_export(&self) -> bool {
        matches!(self, Self::Export)
    }
}

/// 备份口令弹窗的输入与校验状态。
pub(crate) struct BackupPromptState {
    pub(crate) action: BackupAction,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) confirm: Option<Entity<InputState>>,
    pub(crate) encrypt: bool,
    pub(crate) include_secrets: bool,
    pub(crate) encrypted_file: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

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

fn ensure_backup_extension(path: PathBuf) -> PathBuf {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("json" | "yaml" | "yml") => path,
        _ => path.with_extension("json"),
    }
}

mod render;
mod view;

pub(crate) use view::{BackupPromptEvent, BackupPromptView};
