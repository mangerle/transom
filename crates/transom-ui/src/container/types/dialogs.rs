//! 引擎侧栏与工作区共用的弹窗与发现结果状态。

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
    /// Dockerfile 相对路径：镜像构建时使用。
    pub(crate) dockerfile: Option<Entity<InputState>>,
    /// 镜像标签：镜像构建时使用。
    pub(crate) tag: Option<Entity<InputState>>,
    /// 仅宿主机内部可见（`--internal`）。
    pub(crate) internal: bool,
}

/// 一键清理的目标：卷与镜像各自独立待确认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerPruneTarget {
    Volumes,
    Images,
    Networks,
}

impl ContainerPruneTarget {
    /// 工具条按钮文案：清理对象不同，不能共用一句「清理」。
    pub(crate) fn button_label_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.volumes",
            Self::Images => "container.prune.images",
            Self::Networks => "container.prune.networks",
        }
    }

    /// 清理结果的「有删除」与「无删除」文案，避免镜像结果显示成数据卷。
    pub(crate) fn done_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.done",
            Self::Images => "container.prune.images_done",
            Self::Networks => "container.prune.networks_done",
        }
    }

    pub(crate) fn none_key(self) -> &'static str {
        match self {
            Self::Volumes => "container.prune.none",
            Self::Images => "container.prune.images_none",
            Self::Networks => "container.prune.networks_none",
        }
    }
}

/// 引擎发现结果：可添加的引擎与不可用候选的中文摘要。
#[derive(Debug, Clone, Default)]
pub(crate) struct ContainerDiscovery {
    pub(crate) candidates: Vec<ContainerEngineProfile>,
    pub(crate) failures: Vec<String>,
}

/// 引擎连接侧栏与其余容器弹窗的顶层状态。
///
/// 连接表单与运行容器表单已实体化为 `ContainerFormView` / `ContainerRunFormView`，
/// 本结构只保留仍由外壳统一持有的侧栏与标签级弹窗字段。
#[derive(Default)]
pub(crate) struct ContainerUiState {
    pub(crate) sidebar_open: bool,
    pub(crate) fullscreen: Option<TabId>,
    /// 拉取镜像 / 新建卷 / 新建网络的弹窗：字段随分类不同。
    pub(crate) create_dialog: Option<ContainerCreateDialog>,
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
}

/// 引擎分组表单：只编辑名称，父级与“是否重命名”在打开时确定。
pub(crate) struct EngineFolderForm {
    pub(crate) parent: Option<transom_core::FolderId>,
    /// 正在重命名的分组；`None` 表示新建。
    pub(crate) editing: Option<transom_core::FolderId>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
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

    /// 当前平台可用的连接类型。
    ///
    /// Windows 命名管道、WSL 与 WSLC 只在 Windows 出现，
    /// Unix 套接字只在类 Unix 平台出现；SSH 与 TCP 在所有平台均可用。
    pub(crate) fn available() -> Vec<Self> {
        let mut choices = Vec::with_capacity(Self::ALL.len());
        choices.push(Self::Auto);
        if cfg!(unix) {
            choices.push(Self::Unix);
        }
        if cfg!(windows) {
            choices.push(Self::NamedPipe);
        }
        choices.push(Self::Tcp);
        choices.push(Self::Ssh);
        if cfg!(windows) {
            choices.push(Self::Wsl);
            choices.push(Self::Wslc);
        }
        choices.push(Self::Cli);
        choices
    }

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

    /// 主输入框标签；不需要输入的档位返回 `None`。
    pub(crate) fn field_label_key(self) -> Option<&'static str> {
        match self {
            Self::Auto | Self::Wslc => None,
            Self::Unix | Self::NamedPipe => Some("container.form.endpoint_path"),
            Self::Tcp => Some("container.form.endpoint_url"),
            Self::Ssh => Some("container.form.endpoint_ssh_host"),
            Self::Wsl => Some("container.form.endpoint_wsl_distro"),
            Self::Cli => Some("container.form.cli_program"),
        }
    }

    /// 主输入框占位提示。
    pub(crate) fn placeholder_key(self) -> &'static str {
        match self {
            Self::Auto | Self::Wslc => "container.form.endpoint_placeholder",
            Self::Unix => "container.form.unix_placeholder",
            Self::NamedPipe => "container.form.named_pipe_placeholder",
            Self::Tcp => "container.form.tcp_placeholder",
            Self::Ssh => "container.form.ssh_host_placeholder",
            Self::Wsl => "container.form.wsl_placeholder",
            Self::Cli => "container.form.cli_program_placeholder",
        }
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

    /// 表单主输入框的回填值；SSH 端点只回填 host（其余字段由各自输入框承接）。
    pub(crate) fn value_for(endpoint: &ContainerEndpoint) -> String {
        match endpoint {
            ContainerEndpoint::Auto | ContainerEndpoint::Wslc => String::new(),
            ContainerEndpoint::Unix { path } => path.display().to_string(),
            ContainerEndpoint::NamedPipe { path } => path.clone(),
            ContainerEndpoint::Tcp { url, .. } => url.clone(),
            ContainerEndpoint::Ssh { host, .. } => host.clone(),
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

    /// 将表单值组装回端点；SSH 端点的 port/user/identity_file 由调用方单独传入。
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
                tls: None,
            },
            Self::Ssh => ContainerEndpoint::Ssh {
                host: value.to_string(),
                port: None,
                user: None,
                identity_file: None,
                password: None,
                use_sudo: false,
                sudo_password: None,
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
}
