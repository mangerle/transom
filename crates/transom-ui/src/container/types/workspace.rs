//! 单个引擎工作区的状态：列表、选中项、日志、统计、终端与文件页。

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
    /// 正在重命名的源路径；`None` 表示不显示重命名条。
    pub(crate) rename_from: Option<String>,
    /// 重命名输入框；首次点击重命名时创建，之后复用。
    pub(crate) rename_input: Option<Entity<InputState>>,
    /// 新建目录条是否展开。
    pub(crate) new_dir_open: bool,
    /// 新建目录的名字输入框；首次展开时创建，之后复用。
    pub(crate) new_dir_input: Option<Entity<InputState>>,
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
    /// 状态筛选档位（仅容器分类）。
    pub(crate) state_filter: ContainerStateFilter,
    /// 卷与镜像的清理视角。
    pub(crate) quick_filter: ContainerQuickFilter,
    /// 当前排序键与方向。
    pub(crate) sort_key: ContainerSortKey,
    pub(crate) sort_desc: bool,
    /// 三栏拖拽状态：菜单/列表宽度由用户拖动决定，随工作区存活。
    pub(crate) menu_split: Entity<ResizableState>,
    pub(crate) list_split: Entity<ResizableState>,
    pub(crate) containers: Vec<ContainerSummary>,
    pub(crate) images: Vec<ContainerImageSummary>,
    pub(crate) volumes: Vec<ContainerVolumeSummary>,
    pub(crate) networks: Vec<ContainerNetworkSummary>,
    pub(crate) compose: Vec<ComposeProject>,
    /// 当前选中项标识：容器 ID、镜像 ID、卷名、网络 ID 或项目名。
    pub(crate) selected: Option<String>,
    pub(crate) detail_tab: ContainerDetailTab,
    /// 基础信息页签是否切到原始 inspect：两者数据重叠，同一时刻只给一种视图。
    pub(crate) overview_raw: bool,
    pub(crate) details: Option<ContainerDetails>,
    pub(crate) image_details: Option<ContainerImageDetails>,
    pub(crate) volume_details: Option<ContainerVolumeDetails>,
    pub(crate) network_details: Option<ContainerNetworkDetails>,
    pub(crate) logs: String,
    pub(crate) logs_follow: bool,
    /// 日志尾部行数；0 表示全量（引擎侧 `all`）。
    pub(crate) logs_tail: usize,
    /// 是否让引擎附带服务端时间戳。
    pub(crate) logs_timestamps: bool,
    /// 是否把最新一行排在最前：跟随日志时不必手动滚到底。
    pub(crate) logs_reversed: bool,
    pub(crate) logs_request: Option<ContainerRequestId>,
    pub(crate) stats: Option<ContainerStats>,
    /// 历史采样：统计页的走势图参考主机监控，只留最后一帧就只能画一个点。
    pub(crate) stats_history: VecDeque<ContainerStats>,
    /// 走势图形态（折线/柱状），与主机监控同一套组件。
    pub(crate) stats_chart: SparklineChartType,
    /// 采样档位：与主机监控相同的 1s/3s/10s。
    pub(crate) stats_interval: SampleIntervalMode,
    /// 容器进程列表（`ps aux` 口径）。
    pub(crate) stats_processes: Vec<ContainerProcess>,
    pub(crate) stats_processes_request: Option<ContainerRequestId>,
    pub(crate) stats_processes_loading: bool,
    pub(crate) stats_processes_error: Option<String>,
    pub(crate) stats_request: Option<ContainerRequestId>,
    /// 采样失败原因：没有它，页签会永远停在「正在采样统计…」不给任何出口。
    pub(crate) stats_error: Option<String>,
    pub(crate) terminal: Option<ContainerTerminalState>,
    /// 正在建立的终端会话与所选用户：建立前连点页签会重复创建 exec，必须占位拦住。
    pub(crate) terminal_request: Option<(ContainerRequestId, Option<String>)>,
    pub(crate) files: ContainerFilesState,
    /// 多选：勾选的容器 ID，用于批量启停与删除。
    pub(crate) checked: HashSet<String>,
    /// 批量删除的二次确认是否已按下（第一次点只是武装）。
    pub(crate) batch_armed: bool,
    /// 容器改名条：容器 ID 与输入框；`None` 表示不显示。
    pub(crate) rename_container: Option<(String, Entity<InputState>)>,
    /// 跨分类跳转时待选中的容器名：容器列表到达后再选中。
    pub(crate) pending_select: Option<String>,
    /// 待确认的删除动作。
    pub(crate) confirm: Option<ContainerConfirm>,
    /// 已按下一次「清理」、等待第二次确认的目标。
    pub(crate) prune_armed: Option<ContainerPruneTarget>,
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
            state_filter: ContainerStateFilter::All,
            quick_filter: ContainerQuickFilter::All,
            sort_key: ContainerSortKey::default_for(ContainerSection::Containers),
            sort_desc: false,
            menu_split: cx.new(|_| ResizableState::default()),
            list_split: cx.new(|_| ResizableState::default()),
            containers: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
            networks: Vec::new(),
            compose: Vec::new(),
            selected: None,
            detail_tab: ContainerDetailTab::Overview,
            overview_raw: false,
            details: None,
            image_details: None,
            volume_details: None,
            network_details: None,
            logs: String::new(),
            logs_follow: false,
            logs_tail: LOG_TAIL_DEFAULT,
            logs_timestamps: false,
            logs_reversed: false,
            logs_request: None,
            stats: None,
            stats_history: VecDeque::with_capacity(STATS_HISTORY_LIMIT),
            stats_chart: SparklineChartType::default(),
            stats_interval: SampleIntervalMode::Normal,
            stats_processes: Vec::new(),
            stats_processes_request: None,
            stats_processes_loading: false,
            stats_processes_error: None,
            stats_request: None,
            stats_error: None,
            terminal: None,
            terminal_request: None,
            rename_container: None,
            checked: HashSet::default(),
            batch_armed: false,
            pending_select: None,
            files: ContainerFilesState {
                new_dir_open: false,
                new_dir_input: None,
                ..ContainerFilesState::default()
            },
            confirm: None,
            prune_armed: None,
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

    /// 切换某个容器的勾选状态，返回切换后的数量。
    pub(crate) fn toggle_checked(&mut self, id: &str) -> usize {
        toggle_membership(&mut self.checked, id);
        // 勾选集合变了就说明「是否删除全部选中」的确认状态不能再沿用。
        self.batch_armed = false;
        self.checked.len()
    }

    /// 已勾选容器的名字，顺序跟随列表；引擎按名字下发启停与删除。
    pub(crate) fn checked_names(&self) -> Vec<String> {
        let mut names = Vec::with_capacity(self.checked.len());
        for item in &self.containers {
            if self.checked.contains(&item.id) {
                names.push(item.name.clone());
            }
        }
        names
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

    /// 切换分类：排序键要落到该分类可用的档位上，否则会出现「按大小排数据卷」
    /// 这类无效组合；同时收回不可用的详情页签并清掉旧选中项。
    pub(crate) fn apply_section(&mut self, section: ContainerSection) {
        self.section = section;
        // 快捷筛选是分类专属的视角，跨分类留着会继续过滤出莫名其妙的空列表。
        self.quick_filter = if ContainerQuickFilter::for_section(section).len() > 1 {
            self.quick_filter
        } else {
            ContainerQuickFilter::All
        };
        self.selected = None;
        self.details = None;
        self.image_details = None;
        self.volume_details = None;
        self.network_details = None;
        self.detail_tab = ContainerDetailTab::Overview;
        self.overview_raw = false;
        self.stats = None;
        self.stats_history.clear();
        self.stats_error = None;
        self.stats_processes.clear();
        self.stats_processes_error = None;
        self.stats_processes_loading = false;
        self.confirm = None;
        self.prune_armed = None;
        self.checked.clear();
        self.batch_armed = false;
        self.rename_container = None;
        if !ContainerSortKey::for_section(section).contains(&self.sort_key) {
            self.sort_key = ContainerSortKey::default_for(section);
            self.sort_desc = false;
        }
        self.normalize_detail_tab();
    }
}
