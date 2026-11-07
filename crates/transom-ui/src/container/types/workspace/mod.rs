//! 单个引擎工作区的状态：列表、选中项、日志、统计、终端与文件页。
//!
//! 主体状态在 state.rs，文件操作弹窗与确认弹窗类型分别放在 prompt/panel。

mod panel;
mod prompt;
mod state;

pub(crate) use panel::{ContainerCommitState, ContainerConfirm};
pub(crate) use prompt::ContainerFilePrompt;
pub(crate) use state::ContainerWorkspaceState;

use super::*;

/// 工作区连接状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContainerWorkspaceStatus {
    Connecting,
    Connected,
    Failed(String),
    Disconnected,
}

/// 容器内 exec 终端。
pub(crate) struct ContainerTerminalState {
    pub(crate) view: Entity<TerminalView>,
    /// 建立时选择的执行用户：`None` 是镜像默认用户。切换用户要重建会话，
    /// 因此必须记住当前会话用的是谁。
    pub(crate) user: Option<String>,
    /// 建立时选择的 Shell；`None` 是引擎默认（通常为 /bin/sh）。
    pub(crate) shell: Option<String>,
    pub(crate) _subscription: Subscription,
}

/// 文件浏览状态。
#[derive(Default)]
pub(crate) struct ContainerFilesState {
    pub(crate) path: String,
    pub(crate) entries: Vec<ContainerFileEntry>,
    pub(crate) loading: bool,
    /// 文件列表的滚动句柄：切换目录后复位到顶部，列表本身不重建。
    pub(crate) scroll: ScrollHandle,
    /// 在途的目录列表请求：目录切换可能连点，旧请求的结果必须丢弃。
    pub(crate) request: Option<ContainerRequestId>,
    /// 当前预览：路径、内容、是否截断。
    pub(crate) preview: Option<(String, String, bool)>,
    /// 当前预览是否为二进制文件；为真时禁止文本编辑。
    pub(crate) preview_binary: bool,
    /// 就地编辑状态：当用户点击“编辑”时保存该 TextareaState。
    pub(crate) preview_editor: Option<Entity<TextareaState>>,
    /// 编辑器是否正在保存。
    pub(crate) preview_saving: bool,
    /// 文件操作提示词：重命名、复制、改权限或改属主共用一个输入弹窗。
    pub(crate) prompt: Option<ContainerFilePrompt>,
    /// 文件操作输入框；首次打开提示时创建，之后复用。
    pub(crate) prompt_input: Option<Entity<InputState>>,
    /// 新建目录条是否展开。
    pub(crate) new_dir_open: bool,
    /// 新建目录的名字输入框；首次展开时创建，之后复用。
    pub(crate) new_dir_input: Option<Entity<InputState>>,
}
