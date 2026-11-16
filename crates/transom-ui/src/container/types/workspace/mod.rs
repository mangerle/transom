//! 单个引擎工作区的状态：列表、选中项、日志、统计、终端与文件页。
//!
//! 主体状态在 state.rs，文件操作提示、提交/确认与局部弹窗契约聚合在本模块。

mod state;

pub(crate) use state::ContainerWorkspaceState;

use super::*;

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

/// 单个容器引擎工作区内挂靠的模态弹窗状态集合。
#[derive(Default)]
pub(crate) struct ContainerWorkspaceModals {
    pub(crate) confirm: Option<ContainerConfirm>,
    pub(crate) rename_container: Option<(String, Entity<InputState>)>,
    pub(crate) update_container: Option<(String, Entity<InputState>, Entity<InputState>)>,
    pub(crate) tag_image: Option<(String, Entity<InputState>)>,
    pub(crate) connect_network: Option<(String, Entity<InputState>)>,
    pub(crate) export_config: Option<(String, String, String, bool)>,
    pub(crate) commit_container: Option<ContainerCommitState>,
    pub(crate) hub_dialog: Option<ContainerHubDialog>,
    pub(crate) push_dialog: Option<ContainerPushDialog>,
    pub(crate) build_dialog: Option<ContainerBuildDialog>,
    pub(crate) forward_dialog: Option<ContainerForwardDialog>,
    pub(crate) system_df_dialog_open: bool,
    pub(crate) pods_dialog_open: bool,
    pub(crate) quadlets_dialog_open: bool,
}

impl ContainerWorkspaceModals {
    /// 切换工作区分类时收起依赖当前选中对象上下文的轻量弹窗。
    pub(crate) fn clear_on_section_change(&mut self) {
        self.confirm = None;
        self.rename_container = None;
        self.export_config = None;
        self.commit_container = None;
    }
}

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
