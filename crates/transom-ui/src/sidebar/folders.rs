//! 文件夹模态表单数据结构。

use gpui_kit::component::input::InputState;
use gpui_kit::{App, Entity};
use transom_core::FolderId;

/// 新建/编辑文件夹的模态表单；`base` 为编辑目标，`parent` 为新建位置。
pub(crate) struct FolderForm {
    pub(crate) base: Option<FolderId>,
    pub(crate) parent: Option<FolderId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) default_port: Entity<InputState>,
    pub(crate) default_username: Entity<InputState>,
    pub(crate) default_key_path: Entity<InputState>,
    pub(crate) error: Option<String>,
}

impl FolderForm {
    /// 表单全部文本项的当前值，用作脏判定基线（见 `SessionForm::text_values`）。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        [
            &self.name,
            &self.default_port,
            &self.default_username,
            &self.default_key_path,
        ]
        .into_iter()
        .map(|input| input.read(cx).value().to_string())
        .collect()
    }
}
