//! 文件操作输入弹窗的类型与文案键。

/// 文件操作提示词；来源路径与操作类型一起决定标题、校验与命令。
#[derive(Clone)]
pub(crate) enum ContainerFilePrompt {
    Rename { from: String },
    Copy { from: String },
    Chmod { path: String },
    Chown { path: String },
}

impl ContainerFilePrompt {
    pub(crate) fn title_key(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "container.files.rename_modal_title",
            Self::Copy { .. } => "container.files.copy_modal_title",
            Self::Chmod { .. } => "container.files.chmod_modal_title",
            Self::Chown { .. } => "container.files.chown_modal_title",
        }
    }

    pub(crate) fn prompt_key(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "container.files.rename_prompt",
            Self::Copy { .. } => "container.files.copy_prompt",
            Self::Chmod { .. } => "container.files.chmod_prompt",
            Self::Chown { .. } => "container.files.chown_prompt",
        }
    }

    pub(crate) fn placeholder_key(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "container.files.rename_placeholder",
            Self::Copy { .. } => "container.files.copy_placeholder",
            Self::Chmod { .. } => "container.files.chmod_placeholder",
            Self::Chown { .. } => "container.files.chown_placeholder",
        }
    }

    pub(crate) fn default_value(&self) -> String {
        match self {
            Self::Rename { from } => from.rsplit('/').next().unwrap_or(from).to_string(),
            _ => String::new(),
        }
    }
}
