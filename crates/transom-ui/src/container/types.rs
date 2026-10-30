//! 容器管理工作区的界面状态。
//!
//! 设计原理（Why）：工作区状态随标签存活，列表、选中项、详情页签、日志缓冲
//! 与 exec 终端都放在同一个结构里；事件按 `request_id` 找到所属标签后直接
//! 更新对应字段，界面不需要维护额外的订阅表。

use gpui_kit::Entity;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{
    ComposeProject, ContainerDetails, ContainerEndpoint, ContainerEngineInfo, ContainerEngineKind,
    ContainerEngineProfile, ContainerEngineRef, ContainerFileEntry, ContainerImageDetails,
    ContainerImageSummary, ContainerNetworkDetails, ContainerNetworkSummary, ContainerRequestId,
    ContainerStats, ContainerSummary, ContainerVolumeDetails, ContainerVolumeSummary,
    EngineProfileId, TabId,
};
use transom_terminal::TerminalView;

/// 日志缓冲上限：超出后丢弃最旧部分，避免长时间跟随把内存吃满。
pub(crate) const LOG_BUFFER_LIMIT: usize = 512 * 1024;

/// 左侧功能菜单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerSection {
    Compose,
    Containers,
    Images,
    Volumes,
    Networks,
}

impl ContainerSection {
    pub(crate) const ALL: [Self; 5] = [
        Self::Compose,
        Self::Containers,
        Self::Images,
        Self::Volumes,
        Self::Networks,
    ];

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Compose => "container.section.compose",
            Self::Containers => "container.section.containers",
            Self::Images => "container.section.images",
            Self::Volumes => "container.section.volumes",
            Self::Networks => "container.section.networks",
        }
    }

    pub(crate) fn icon(self) -> gpui_kit::assets::IconName {
        use gpui_kit::assets::IconName;
        match self {
            Self::Compose => IconName::Boxes,
            Self::Containers => IconName::Container,
            Self::Images => IconName::Layers,
            Self::Volumes => IconName::Database,
            Self::Networks => IconName::Network,
        }
    }

    /// 详情页签在该分类下是否有意义。
    pub(crate) fn detail_tabs(self) -> &'static [ContainerDetailTab] {
        match self {
            Self::Containers => &[
                ContainerDetailTab::Overview,
                ContainerDetailTab::Logs,
                ContainerDetailTab::Terminal,
                ContainerDetailTab::Stats,
                ContainerDetailTab::Files,
            ],
            Self::Compose => &[ContainerDetailTab::Overview],
            Self::Images => &[ContainerDetailTab::Overview],
            Self::Volumes => &[ContainerDetailTab::Overview],
            Self::Networks => &[ContainerDetailTab::Overview],
        }
    }
}

/// 右侧详情页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerDetailTab {
    Overview,
    Logs,
    Terminal,
    Stats,
    Files,
}

impl ContainerDetailTab {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Overview => "container.detail.overview",
            Self::Logs => "container.detail.logs",
            Self::Terminal => "container.detail.terminal",
            Self::Stats => "container.detail.stats",
            Self::Files => "container.detail.files",
        }
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
    pub(crate) _subscription: Subscription,
}

/// 文件浏览状态。
#[derive(Default)]
pub(crate) struct ContainerFilesState {
    pub(crate) path: String,
    pub(crate) entries: Vec<ContainerFileEntry>,
    pub(crate) loading: bool,
    /// 当前预览：路径、内容、是否截断。
    pub(crate) preview: Option<(String, String, bool)>,
    /// 正在重命名的源路径；`None` 表示不显示重命名条。
    pub(crate) rename_from: Option<String>,
    /// 重命名输入框；首次点击重命名时创建，之后复用。
    pub(crate) rename_input: Option<Entity<InputState>>,
}

/// 待确认的删除操作：删除是破坏性动作，必须再点一次确认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContainerConfirm {
    Container(String),
    Image(String),
    Volume(String),
    Network(String),
    FilePath { path: String, recursive: bool },
}

/// 一个引擎工作区的全部界面状态。
pub(crate) struct ContainerWorkspaceState {
    pub(crate) profile: ContainerEngineProfile,
    pub(crate) status: ContainerWorkspaceStatus,
    pub(crate) info: Option<ContainerEngineInfo>,
    pub(crate) section: ContainerSection,
    pub(crate) search: Entity<InputState>,
    /// 拉取镜像/新建卷/新建网络的单行输入。
    pub(crate) create_input: Entity<InputState>,
    pub(crate) running_only: bool,
    pub(crate) containers: Vec<ContainerSummary>,
    pub(crate) images: Vec<ContainerImageSummary>,
    pub(crate) volumes: Vec<ContainerVolumeSummary>,
    pub(crate) networks: Vec<ContainerNetworkSummary>,
    pub(crate) compose: Vec<ComposeProject>,
    /// 当前选中项标识：容器 ID、镜像 ID、卷名、网络 ID 或项目名。
    pub(crate) selected: Option<String>,
    pub(crate) detail_tab: ContainerDetailTab,
    pub(crate) details: Option<ContainerDetails>,
    pub(crate) image_details: Option<ContainerImageDetails>,
    pub(crate) volume_details: Option<ContainerVolumeDetails>,
    pub(crate) network_details: Option<ContainerNetworkDetails>,
    pub(crate) logs: String,
    pub(crate) logs_follow: bool,
    pub(crate) logs_request: Option<ContainerRequestId>,
    pub(crate) stats: Option<ContainerStats>,
    pub(crate) stats_request: Option<ContainerRequestId>,
    pub(crate) terminal: Option<ContainerTerminalState>,
    pub(crate) files: ContainerFilesState,
    /// 待确认的删除动作。
    pub(crate) confirm: Option<ContainerConfirm>,
    /// 最近一条操作结果/错误提示。
    pub(crate) notice: Option<String>,
    pub(crate) busy: bool,
    /// 搜索框变化时触发整壳重绘，过滤结果随输入实时更新。
    pub(crate) _search_subscription: Subscription,
    pub(crate) _create_subscription: Subscription,
}

impl ContainerWorkspaceState {
    pub(crate) fn new(
        profile: ContainerEngineProfile,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("container.search").to_string())
        });
        let create_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.create_placeholder").to_string())
        });
        let search_subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let create_subscription = cx.subscribe(&create_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            profile,
            status: ContainerWorkspaceStatus::Connecting,
            info: None,
            section: ContainerSection::Containers,
            search,
            create_input,
            running_only: false,
            containers: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
            networks: Vec::new(),
            compose: Vec::new(),
            selected: None,
            detail_tab: ContainerDetailTab::Overview,
            details: None,
            image_details: None,
            volume_details: None,
            network_details: None,
            logs: String::new(),
            logs_follow: false,
            logs_request: None,
            stats: None,
            stats_request: None,
            terminal: None,
            files: ContainerFilesState::default(),
            confirm: None,
            notice: None,
            busy: false,
            _search_subscription: search_subscription,
            _create_subscription: create_subscription,
        }
    }

    pub(crate) fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    pub(crate) fn append_logs(&mut self, bytes: &[u8]) {
        self.logs.push_str(&String::from_utf8_lossy(bytes));
        if self.logs.len() > LOG_BUFFER_LIMIT {
            let mut cut = self.logs.len() - LOG_BUFFER_LIMIT;
            while cut < self.logs.len() && !self.logs.is_char_boundary(cut) {
                cut += 1;
            }
            self.logs.drain(..cut);
        }
    }

    pub(crate) fn selected_container(&self) -> Option<&ContainerSummary> {
        let id = self.selected.as_deref()?;
        self.containers.iter().find(|item| item.id == id)
    }

    pub(crate) fn selected_container_name(&self) -> Option<String> {
        self.selected_container().map(|item| item.name.clone())
    }

    pub(crate) fn selected_image(&self) -> Option<&ContainerImageSummary> {
        let id = self.selected.as_deref()?;
        self.images.iter().find(|item| item.id == id)
    }

    pub(crate) fn selected_volume(&self) -> Option<&ContainerVolumeSummary> {
        let name = self.selected.as_deref()?;
        self.volumes.iter().find(|item| item.name == name)
    }

    pub(crate) fn selected_network(&self) -> Option<&ContainerNetworkSummary> {
        let id = self.selected.as_deref()?;
        self.networks.iter().find(|item| item.id == id)
    }

    pub(crate) fn selected_compose(&self) -> Option<&ComposeProject> {
        let name = self.selected.as_deref()?;
        self.compose.iter().find(|item| item.name == name)
    }

    /// 详情页签在当前分类下不可用时回落到基础信息。
    pub(crate) fn normalize_detail_tab(&mut self) {
        let available = self.section.detail_tabs();
        if !available.contains(&self.detail_tab) {
            self.detail_tab = ContainerDetailTab::Overview;
        }
    }
}

/// 引擎连接侧栏与表单的顶层状态。
#[derive(Default)]
pub(crate) struct ContainerUiState {
    pub(crate) sidebar_open: bool,
    pub(crate) form: Option<ContainerFormState>,
    pub(crate) fullscreen: Option<TabId>,
    /// 请求 ID → 所属工作区标签；事件按它路由。
    pub(crate) requests: HashMap<ContainerRequestId, TabId>,
}

impl ContainerUiState {
    pub(crate) fn track(&mut self, request_id: ContainerRequestId, tab_id: TabId) {
        self.requests.insert(request_id, tab_id);
    }

    pub(crate) fn take_tab(&mut self, request_id: ContainerRequestId) -> Option<TabId> {
        self.requests.remove(&request_id)
    }

    pub(crate) fn tab_of(&self, request_id: ContainerRequestId) -> Option<TabId> {
        self.requests.get(&request_id).copied()
    }
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

/// 容器表单状态。
pub(crate) struct ContainerFormState {
    pub(crate) editing: Option<EngineProfileId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) kind: ContainerEngineKind,
    pub(crate) endpoint_choice: EndpointChoice,
    pub(crate) endpoint_value: Entity<InputState>,
    /// 测试连接结果提示。
    pub(crate) test_status: Option<String>,
    pub(crate) test_ok: bool,
    pub(crate) test_request: Option<ContainerRequestId>,
}

impl ContainerFormState {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
        profile: Option<&ContainerEngineProfile>,
    ) -> Self {
        let name_value = profile
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let engine = profile
            .map(|profile| profile.engine.clone())
            .unwrap_or_else(|| ContainerEngineRef::auto(ContainerEngineKind::Docker));
        let endpoint_choice = EndpointChoice::from_endpoint(&engine.endpoint);
        let endpoint_value = EndpointChoice::value_for(&engine.endpoint);
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name_value)
                .placeholder(rust_i18n::t!("container.form.name_placeholder").to_string())
        });
        let endpoint = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(endpoint_value)
                .placeholder(rust_i18n::t!("container.form.endpoint_placeholder").to_string())
        });
        Self {
            editing: profile.map(|profile| profile.id),
            name,
            kind: engine.kind,
            endpoint_choice,
            endpoint_value: endpoint,
            test_status: None,
            test_ok: false,
            test_request: None,
        }
    }

    /// 构造保存/测试用的连接配置。
    ///
    /// # Errors
    /// 名称为空、需要端点值却留空，或端点在当前平台不可用时返回中文提示。
    pub(crate) fn to_profile(&self, cx: &App) -> Result<ContainerEngineProfile, String> {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(rust_i18n::t!("container.form.name_required").to_string());
        }
        let value = self.endpoint_value.read(cx).value().to_string();
        if self.endpoint_choice.needs_value() && value.trim().is_empty() {
            return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
        }
        let endpoint = self.endpoint_choice.to_endpoint(&value);
        endpoint
            .ensure_supported()
            .map_err(|reason| reason.to_string())?;
        let engine = ContainerEngineRef::explicit(self.kind, endpoint);
        let mut profile = ContainerEngineProfile::new(name, engine);
        if let Some(id) = self.editing {
            profile.id = id;
        }
        Ok(profile)
    }
}
