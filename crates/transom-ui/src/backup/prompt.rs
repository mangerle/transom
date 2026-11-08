//! 备份弹窗的纯数据契约：动作方向与一次打开期的输入/校验状态。
//!
//! 渲染归 [`super::render`]，任务与生命周期归 [`super::view`]；这里只保留
//! 可被两者共同读取的状态，避免把“弹窗长什么样”和“弹窗拥有什么”混在一处。

use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use std::path::PathBuf;

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
///
/// `encrypt`/`include_secrets`/`encrypted_file` 分别是用户选择、导出内容
/// 范围与所选文件自身格式，三者语义不同，不能合并成互斥枚举。
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
