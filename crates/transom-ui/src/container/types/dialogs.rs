//! 引擎侧栏与工作区共用的弹窗、发现结果与请求登记状态。

use super::*;

/// 拉取/新建弹窗的状态。
///
/// 设计原理（Why）：原先把输入框常驻在中栏底部，三个分类各占一行高度却几乎
/// 用不到（真实窗口实测：用户要求去掉底部新增行）。改成按需弹窗后，主输入仍
/// 复用工作区的 `create_input`（校验与提交路径不变），弹窗只多带一个驱动输入。
#[derive(Clone)]
pub(crate) struct ContainerCreateDialog {
    pub(crate) tab_id: TabId,
    pub(crate) section: ContainerSection,
    /// 驱动输入：数据卷与网络有，镜像拉取没有。
    pub(crate) driver: Option<Entity<InputState>>,
    /// 子网与网关：只有网络需要；留空由引擎分配。
    pub(crate) subnet: Option<Entity<InputState>>,
    pub(crate) gateway: Option<Entity<InputState>>,
    /// 仅宿主机内部可见（`--internal`）。
    pub(crate) internal: bool,
}

/// 一键清理的目标：卷与镜像各自独立待确认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerPruneTarget {
    Volumes,
    Images,
}

impl ContainerPruneTarget {
    /// 工具条按钮文案：清理对象不同，不能共用一句「清理」。
    pub(crate) fn button_label_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.volumes",
            Self::Images => "container.prune.images",
        }
    }

    pub(crate) fn hint_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.hint",
            Self::Images => "container.prune.images_hint",
        }
    }

    /// 清理结果的「有删除」与「无删除」文案，避免镜像结果显示成数据卷。
    pub(crate) fn done_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.done",
            Self::Images => "container.prune.images_done",
        }
    }

    pub(crate) fn none_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.none",
            Self::Images => "container.prune.images_none",
        }
    }
}

/// 引擎发现结果：可添加的引擎与不可用候选的中文摘要。
#[derive(Debug, Clone, Default)]
pub(crate) struct ContainerDiscovery {
    pub(crate) candidates: Vec<ContainerEngineProfile>,
    pub(crate) failures: Vec<String>,
}

/// 引擎连接侧栏与表单的顶层状态。
#[derive(Default)]
pub(crate) struct ContainerUiState {
    pub(crate) sidebar_open: bool,
    pub(crate) form: Option<ContainerFormState>,
    pub(crate) fullscreen: Option<TabId>,
    /// 运行容器表单。
    pub(crate) run_form: Option<ContainerRunForm>,
    /// 拉取镜像 / 新建卷 / 新建网络的弹窗：字段随分类不同。
    pub(crate) create_dialog: Option<ContainerCreateDialog>,
    /// 请求 ID → 所属工作区标签与动作描述；事件按它路由。
    pub(crate) requests: HashMap<ContainerRequestId, ContainerRequest>,
    /// 最近一次「发现本机引擎」的结果面板。
    pub(crate) discovery: Option<ContainerDiscovery>,
    /// 正在进行的发现请求，避免连点重复探测。
    pub(crate) discovering: bool,
    /// 侧栏当前单选高亮的引擎连接。
    pub(crate) sidebar_selected: Option<transom_core::EngineProfileId>,
    /// 侧栏搜索框：首次渲染时创建，随后复用。
    pub(crate) sidebar_search: Option<Entity<InputState>>,
    /// 引擎分组的新建/重命名弹窗。
    pub(crate) engine_folder_form: Option<EngineFolderForm>,
    /// 从分组菜单新建连接时暂存目标分组；保存成功后落到新连接上。
    pub(crate) form_folder: Option<transom_core::FolderId>,
}

/// 引擎分组表单：只编辑名称，父级与“是否重命名”在打开时确定。
pub(crate) struct EngineFolderForm {
    pub(crate) parent: Option<transom_core::FolderId>,
    /// 正在重命名的分组；`None` 表示新建。
    pub(crate) editing: Option<transom_core::FolderId>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

impl ContainerUiState {
    pub(crate) fn track(
        &mut self,
        request_id: ContainerRequestId,
        tab_id: TabId,
        label: Option<String>,
    ) {
        self.requests.insert(
            request_id,
            ContainerRequest {
                tab_id,
                label: label.filter(|label| !label.is_empty()),
            },
        );
    }

    pub(crate) fn take_tab(&mut self, request_id: ContainerRequestId) -> Option<TabId> {
        self.requests
            .remove(&request_id)
            .map(|request| request.tab_id)
    }

    pub(crate) fn tab_of(&self, request_id: ContainerRequestId) -> Option<TabId> {
        self.requests.get(&request_id).map(|request| request.tab_id)
    }

    /// 取回发送时登记的动作描述（不消费记录）：结果事件用它拼提示文案。
    pub(crate) fn label_of(&self, request_id: ContainerRequestId) -> Option<String> {
        self.requests
            .get(&request_id)
            .and_then(|request| request.label.clone())
    }
}

/// 一条在途请求的路由信息。
///
/// 设计原理（Why）：结果事件只带 `request_id`，要给出「启动 web-1 成功」这样的
/// 可读提示，就得在发送时把动作描述一起登记；只读请求（列表、inspect、日志）
/// 的 `label` 为空，成功时不弹提示，避免每次刷新都糊一条 toast。
pub(crate) struct ContainerRequest {
    pub(crate) tab_id: TabId,
    pub(crate) label: Option<String>,
}

/// 端点输入档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum EndpointChoice {
    #[default]
    Auto,
    Unix,
    NamedPipe,
    Tcp,
    Ssh,
    Wsl,
    Wslc,
    Cli,
}

impl EndpointChoice {
    pub(crate) const ALL: [Self; 8] = [
        Self::Auto,
        Self::Unix,
        Self::NamedPipe,
        Self::Tcp,
        Self::Ssh,
        Self::Wsl,
        Self::Wslc,
        Self::Cli,
    ];

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Auto => "container.endpoint.auto",
            Self::Unix => "container.endpoint.unix",
            Self::NamedPipe => "container.endpoint.named_pipe",
            Self::Tcp => "container.endpoint.tcp",
            Self::Ssh => "container.endpoint.ssh",
            Self::Wsl => "container.endpoint.wsl",
            Self::Wslc => "container.endpoint.wslc",
            Self::Cli => "container.endpoint.cli",
        }
    }

    /// 该档位是否需要填写附加输入。
    pub(crate) fn needs_value(self) -> bool {
        !matches!(self, Self::Auto | Self::Wslc)
    }

    pub(crate) fn from_endpoint(endpoint: &ContainerEndpoint) -> Self {
        match endpoint {
            ContainerEndpoint::Auto => Self::Auto,
            ContainerEndpoint::Unix { .. } => Self::Unix,
            ContainerEndpoint::NamedPipe { .. } => Self::NamedPipe,
            ContainerEndpoint::Tcp { .. } => Self::Tcp,
            ContainerEndpoint::Ssh { .. } => Self::Ssh,
            ContainerEndpoint::Wsl { .. } => Self::Wsl,
            ContainerEndpoint::Wslc => Self::Wslc,
            ContainerEndpoint::Cli { .. } => Self::Cli,
        }
    }

    /// 表单值转端点；空值由调用方在保存前校验。
    pub(crate) fn to_endpoint(self, value: &str) -> ContainerEndpoint {
        let value = value.trim();
        match self {
            Self::Auto => ContainerEndpoint::Auto,
            Self::Unix => ContainerEndpoint::Unix { path: value.into() },
            Self::NamedPipe => ContainerEndpoint::NamedPipe {
                path: value.to_string(),
            },
            Self::Tcp => ContainerEndpoint::Tcp {
                url: value.to_string(),
            },
            Self::Ssh => ContainerEndpoint::Ssh {
                target: value.trim_start_matches("ssh://").to_string(),
            },
            Self::Wsl => ContainerEndpoint::Wsl {
                distro: value.to_string(),
            },
            Self::Wslc => ContainerEndpoint::Wslc,
            Self::Cli => ContainerEndpoint::Cli {
                program: value.split_whitespace().next().unwrap_or("").to_string(),
                args: value
                    .split_whitespace()
                    .skip(1)
                    .map(str::to_string)
                    .collect(),
            },
        }
    }

    /// 端点的可编辑文本；仅用于表单回填。
    pub(crate) fn value_for(endpoint: &ContainerEndpoint) -> String {
        match endpoint {
            ContainerEndpoint::Auto | ContainerEndpoint::Wslc => String::new(),
            ContainerEndpoint::Unix { path } => path.display().to_string(),
            ContainerEndpoint::NamedPipe { path } | ContainerEndpoint::Tcp { url: path } => {
                path.clone()
            }
            ContainerEndpoint::Ssh { target } => target.clone(),
            ContainerEndpoint::Wsl { distro } => distro.clone(),
            ContainerEndpoint::Cli { program, args } => {
                let mut value = program.clone();
                for arg in args {
                    value.push(' ');
                    value.push_str(arg);
                }
                value
            }
        }
    }
}
