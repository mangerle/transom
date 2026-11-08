//! 会话目录备份的导入导出交互。
//!
//! 加密与否由用户在弹窗里勾选：加密备份用 Argon2id 派生密钥并以
//! XChaCha20-Poly1305 认证加密写入信封；明文备份直接落一份 JSON/YAML，
//! 便于人工查看与迁移，但文件里的主机名、用户名是明文。
//!
//! 弹窗状态、后台任务与渲染归 [`BackupPromptView`] 实体；本模块只做重导出。

mod helpers;
mod prompt;
mod render;
mod view;

pub(crate) use helpers::secret_restore_note;
pub(crate) use view::{BackupPromptEvent, BackupPromptView};
