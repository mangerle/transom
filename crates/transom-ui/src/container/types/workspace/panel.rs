//! 提交弹窗与操作确认的界面状态。

use super::super::*;

/// 容器提交镜像的就地表单状态。
#[derive(Clone)]
pub(crate) struct ContainerCommitState {
    pub(crate) container: String,
    pub(crate) repo: Entity<InputState>,
    pub(crate) tag: Entity<InputState>,
    pub(crate) comment: Entity<InputState>,
}

/// 待确认的删除操作：删除是破坏性动作，必须再点一次确认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContainerConfirm {
    Container(String),
    Image(String),
    Volume(String),
    Network(String),
    /// 编排项目 `down`：会删掉该项目创建的容器与网络。
    Compose(String),
    FilePath {
        path: String,
        recursive: bool,
    },
    /// 上传目标已存在：确认后覆盖。
    UploadOverwrite {
        path: String,
        local_path: std::path::PathBuf,
        directory: bool,
    },
}
