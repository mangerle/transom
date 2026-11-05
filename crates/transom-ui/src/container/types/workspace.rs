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
    /// 就地编辑状态：当用户点击“编辑”时保存该 TextareaState。
    pub(crate) preview_editor: Option<Entity<TextareaState>>,
    /// 编辑器是否正在保存。
    pub(crate) preview_saving: bool,
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
    /// 是否在「容器」列表中包含由 Compose 启动的衍生容器（默认 false）。
    pub(crate) include_compose: bool,
    /// 卷与镜像的清理视角。
    pub(crate) quick_filter: ContainerQuickFilter,
    /// 当前排序键与方向。
    pub(crate) sort_key: ContainerSortKey,
    pub(crate) sort_desc: bool,
    /// 列表栏拖拽状态：宽度由用户拖动决定，随工作区存活。
    pub(crate) list_split: Entity<ResizableState>,
    /// 列表键盘导航焦点句柄。
    pub(crate) list_focus_handle: FocusHandle,
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
    /// 原始 inspect 搜索框与订阅。
    pub(crate) inspect_search: Entity<InputState>,
    pub(crate) _inspect_search_subscription: Subscription,
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
    /// 上一次把统计样本追加到 UI 的时间；用于按档位节流，不影响后台统计流。
    pub(crate) stats_last_append: Option<Instant>,
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
    /// 统计看板端口映射与进程表格的折叠状态。
    pub(crate) stats_ports_collapsed: bool,
    pub(crate) stats_processes_collapsed: bool,
    pub(crate) terminal: Option<ContainerTerminalState>,
    /// 正在建立的终端会话与所选 (用户, Shell)：建立前连点页签会重复创建 exec，必须占位拦住。
    pub(crate) terminal_request: Option<(ContainerRequestId, Option<String>, Option<String>)>,
    pub(crate) files: ContainerFilesState,
    pub(crate) changes: Option<(String, Vec<transom_core::ContainerFileSystemChange>)>,
    pub(crate) changes_loading: bool,
    pub(crate) changes_request: Option<ContainerRequestId>,
    pub(crate) changes_search: Entity<InputState>,
    pub(crate) _changes_search_subscription: Subscription,
    /// 容器改名条：容器 ID 与输入框；`None` 表示不显示。
    pub(crate) rename_container: Option<(String, Entity<InputState>)>,
    /// 容器配置更新条：容器名与 (内存MB输入框, CPU输入框)；`None` 表示不显示。
    pub(crate) update_container: Option<(String, Entity<InputState>, Entity<InputState>)>,
    /// 镜像打标签条：镜像 ID 与新 tag 输入框；`None` 表示不显示。
    pub(crate) tag_image: Option<(String, Entity<InputState>)>,
    /// 网络连接容器条：网络 ID 与待连接容器名输入框；`None` 表示不显示。
    pub(crate) connect_network: Option<(String, Entity<InputState>)>,
    /// 容器配置导出条：(容器名, run命令, compose YAML, 是否展示compose)；`None` 表示不显示。
    pub(crate) export_config: Option<(String, String, String, bool)>,
    /// 容器提交镜像条：(容器名, repo输入框, tag输入框, comment输入框)；`None` 表示不显示。
    pub(crate) commit_container: Option<(
        String,
        Entity<InputState>,
        Entity<InputState>,
        Entity<InputState>,
    )>,
    /// 跨分类跳转时待选中的容器名：容器列表到达后再选中。
    pub(crate) pending_select: Option<String>,
    /// 待确认的删除动作。
    pub(crate) confirm: Option<ContainerConfirm>,
    /// 已按下一次「清理」、等待第二次确认的目标。
    /// 镜像下载管理器：有界队列与分层进度。
    pub(crate) pull_manager: crate::container::pull_manager::ImagePullManager,
    /// 下载管理抽屉是否展开。
    pub(crate) pull_drawer_open: bool,
    /// 镜像市场探索弹窗状态。
    pub(crate) hub_dialog: Option<ContainerHubDialog>,
    /// 镜像推送到远端仓库弹窗状态。
    pub(crate) push_dialog: Option<ContainerPushDialog>,
    /// 从 Dockerfile 构建镜像弹窗状态。
    pub(crate) build_dialog: Option<ContainerBuildDialog>,
    /// 容器端口建立本地端口转发弹窗状态。
    pub(crate) forward_dialog: Option<ContainerForwardDialog>,
    pub(crate) prune_armed: Option<ContainerPruneTarget>,
    /// 系统全局磁盘用量分析数据。
    pub(crate) system_df: Option<transom_core::ContainerSystemDiskUsage>,
    pub(crate) system_df_loading: bool,
    pub(crate) system_df_request: Option<ContainerRequestId>,
    pub(crate) system_df_dialog_open: bool,
    pub(crate) system_prune_all: bool,
    pub(crate) system_prune_volumes: bool,
    pub(crate) system_prune_running: bool,
    /// Compose 项目的 YAML 编辑状态：(项目名, 配置文件路径, 编辑器 Entity, 是否正在保存)。
    pub(crate) compose_yaml_editor:
        Option<(String, std::path::PathBuf, Entity<TextareaState>, bool)>,
    /// Compose 项目的微服务依赖图：(服务名 -> 上游依赖的服务名列表)。
    pub(crate) compose_dependencies: HashMap<String, Vec<String>>,
    /// 拓扑看板中当前鼠标悬停的服务名；用于上下游依赖卡片与连线的高亮聚焦。
    pub(crate) compose_hovered_service: Option<String>,
    /// 待展示的操作提示；渲染帧消费后转为右上角通知并清空。
    pub(crate) notice: Option<String>,
    pub(crate) busy: bool,
    /// 搜索框变化时触发整壳重绘，过滤结果随输入实时更新。
    pub(crate) _search_subscription: Subscription,
    pub(crate) _create_subscription: Subscription,
    /// 日志关键字过滤输入框与订阅。
    pub(crate) logs_search: Entity<InputState>,
    pub(crate) _logs_search_subscription: Subscription,
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
        let logs_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.logs.search_placeholder").to_string())
        });
        let logs_search_subscription = cx.subscribe(&logs_search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let inspect_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.inspect.search_placeholder").to_string())
        });
        let inspect_search_subscription =
            cx.subscribe(&inspect_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let changes_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.changes.search_placeholder").to_string())
        });
        let changes_search_subscription =
            cx.subscribe(&changes_search, |_, _, event: &InputEvent, cx| {
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
            include_compose: false,
            quick_filter: ContainerQuickFilter::All,
            sort_key: ContainerSortKey::default_for(ContainerSection::Containers),
            sort_desc: false,
            list_split: cx.new(|_| ResizableState::default()),
            list_focus_handle: cx.focus_handle(),
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
            stats_last_append: None,
            stats_chart: SparklineChartType::default(),
            stats_interval: SampleIntervalMode::Normal,
            stats_processes: Vec::new(),
            stats_processes_request: None,
            stats_processes_loading: false,
            stats_processes_error: None,
            stats_request: None,
            stats_error: None,
            stats_ports_collapsed: false,
            stats_processes_collapsed: false,
            terminal: None,
            terminal_request: None,
            rename_container: None,
            update_container: None,
            tag_image: None,
            connect_network: None,
            export_config: None,
            commit_container: None,
            pending_select: None,
            files: ContainerFilesState {
                new_dir_open: false,
                new_dir_input: None,
                ..ContainerFilesState::default()
            },
            confirm: None,
            pull_manager: crate::container::pull_manager::ImagePullManager::new(),
            pull_drawer_open: false,
            hub_dialog: None,
            push_dialog: None,
            build_dialog: None,
            forward_dialog: None,
            prune_armed: None,
            system_df: None,
            system_df_loading: false,
            system_df_request: None,
            system_df_dialog_open: false,
            system_prune_all: false,
            system_prune_volumes: false,
            system_prune_running: false,
            compose_yaml_editor: None,
            compose_dependencies: HashMap::new(),
            compose_hovered_service: None,
            notice: None,
            busy: false,
            _search_subscription: search_subscription,
            _create_subscription: create_subscription,
            logs_search,
            _logs_search_subscription: logs_search_subscription,
            inspect_search,
            _inspect_search_subscription: inspect_search_subscription,
            changes: None,
            changes_loading: false,
            changes_request: None,
            changes_search,
            _changes_search_subscription: changes_search_subscription,
        }
    }

    pub(crate) fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    pub(crate) fn logs_search_query(&self, cx: &App) -> String {
        self.logs_search.read(cx).value().to_string()
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

    /// 获取当前引擎端点唤起浏览器所访问的目标主机地址。
    pub(crate) fn browser_host(&self) -> String {
        self.profile.engine.browser_host()
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

    pub(crate) fn selected_volume_name(&self) -> Option<String> {
        self.selected_volume().map(|item| item.name.clone())
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
        self.include_compose = false;
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
        self.stats_last_append = None;
        self.stats_error = None;
        self.stats_processes.clear();
        self.stats_processes_error = None;
        self.stats_processes_loading = false;
        self.confirm = None;
        self.prune_armed = None;
        self.rename_container = None;
        self.export_config = None;
        self.commit_container = None;
        self.compose_hovered_service = None;
        self.compose_dependencies.clear();
        if !ContainerSortKey::for_section(section).contains(&self.sort_key) {
            self.sort_key = ContainerSortKey::default_for(section);
            self.sort_desc = false;
        }
        self.normalize_detail_tab();
    }
}
