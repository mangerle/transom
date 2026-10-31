//! 容器管理工作区的界面状态。
//!
//! 设计原理（Why）：工作区状态随标签存活，列表、选中项、详情页签、日志缓冲
//! 与 exec 终端都放在同一个结构里；事件按 `request_id` 找到所属标签后直接
//! 更新对应字段，界面不需要维护额外的订阅表。

use crate::monitor::{SampleIntervalMode, SparklineChartType};
use gpui_kit::Entity;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::{HashMap, HashSet, VecDeque};
use transom_core::{
    ComposeProject, ContainerCreateSpec, ContainerDetails, ContainerEndpoint, ContainerEngineInfo,
    ContainerEngineKind, ContainerEngineProfile, ContainerEngineRef, ContainerFileEntry,
    ContainerImageDetails, ContainerImageSummary, ContainerNetworkDetails, ContainerNetworkSummary,
    ContainerPortPublish, ContainerProcess, ContainerRequestId, ContainerStats, ContainerSummary,
    ContainerVolumeDetails, ContainerVolumeSummary, EngineProfileId, TabId,
};
use transom_terminal::TerminalView;

/// 日志缓冲上限：超出后丢弃最旧部分，避免长时间跟随把内存吃满。
pub(crate) const LOG_BUFFER_LIMIT: usize = 512 * 1024;

/// 统计历史窗口：与主机监控走势图槽位一致，约两分钟（3 秒采样）。
pub(crate) const STATS_HISTORY_LIMIT: usize = 40;
/// 默认只取尾部 500 行：一次性拉全量日志会把大容器的输出灌进界面缓冲。
pub(crate) const LOG_TAIL_DEFAULT: usize = 500;
/// 日志尾部行数档位：与 Portainer/lazydocker 的取数习惯一致。
pub(crate) const LOG_TAIL_CHOICES: [usize; 4] = [100, LOG_TAIL_DEFAULT, 5000, 0];

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

/// 容器状态筛选档位。
///
/// 设计原理（Why）：原先只有「仅运行中」一个开关，想看已退出的一次性容器就
/// 只能取消筛选再肉眼找；Portainer/Docker Desktop 都是三档，且默认全部。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerStateFilter {
    All,
    Running,
    Stopped,
}

impl ContainerStateFilter {
    pub(crate) const ALL: [Self; 3] = [Self::All, Self::Running, Self::Stopped];

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::All => "container.filter.all",
            Self::Running => "container.filter.running",
            Self::Stopped => "container.filter.stopped",
        }
    }

    /// 运行态是否落在该档位内。
    pub(crate) fn accepts(self, running: bool) -> bool {
        match self {
            Self::All => true,
            Self::Running => running,
            Self::Stopped => !running,
        }
    }
}

/// 快捷筛选：卷与镜像的「清理视角」。
///
/// 设计原理（Why）：`docker volume ls --filter dangling=true` 与「悬空镜像」是
/// 清理磁盘时的第一需求，界面上没有它就得靠用户在几百条里肉眼比对；这里复用已经
/// 拉到本地的列表计算，不额外发请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ContainerQuickFilter {
    #[default]
    All,
    /// 未被任何容器挂载的卷。
    UnusedVolumes,
    /// 没有任何标签的镜像。
    DanglingImages,
}

impl ContainerQuickFilter {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::All => "container.filter.all",
            Self::UnusedVolumes => "container.filter.unused",
            Self::DanglingImages => "container.filter.dangling",
        }
    }

    /// 该分类可用的快捷筛选；容器与网络没有这类清理语义。
    pub(crate) fn for_section(section: ContainerSection) -> &'static [Self] {
        match section {
            ContainerSection::Volumes => &[Self::All, Self::UnusedVolumes],
            ContainerSection::Images => &[Self::All, Self::DanglingImages],
            ContainerSection::Containers
            | ContainerSection::Networks
            | ContainerSection::Compose => &[Self::All],
        }
    }
}

/// 列表排序键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerSortKey {
    Name,
    State,
    Created,
    Size,
}

impl ContainerSortKey {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Name => "container.sort.name",
            Self::State => "container.sort.state",
            Self::Created => "container.sort.created",
            Self::Size => "container.sort.size",
        }
    }

    /// 该分类可用的排序键：镜像没有状态、卷没有大小字段，不给无效选项。
    pub(crate) fn for_section(section: ContainerSection) -> &'static [Self] {
        use ContainerSortKey::{Created, Name, Size, State};
        match section {
            ContainerSection::Containers => &[Name, State, Created],
            ContainerSection::Compose => &[Name, State],
            ContainerSection::Images => &[Name, Created, Size],
            ContainerSection::Volumes => &[Name, Created],
            ContainerSection::Networks => &[Name],
        }
    }

    /// 该分类的默认排序键。
    pub(crate) fn default_for(section: ContainerSection) -> Self {
        Self::for_section(section)[0]
    }
}

/// 排序所需的全部字段：把五种条目统一到同一个比较器上。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SortFields {
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) created: i64,
    pub(crate) size: u64,
}

/// 按排序键比较两条条目；降序时取反，名称永远作为最后的稳定 tie-break。
pub(crate) fn compare_fields(
    left: &SortFields,
    right: &SortFields,
    key: ContainerSortKey,
    desc: bool,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let primary = match key {
        ContainerSortKey::Name => Ordering::Equal,
        ContainerSortKey::State => left
            .state
            .to_ascii_lowercase()
            .cmp(&right.state.to_ascii_lowercase()),
        ContainerSortKey::Created => left.created.cmp(&right.created),
        ContainerSortKey::Size => left.size.cmp(&right.size),
    };
    let ordered = match primary {
        Ordering::Equal => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        other => other,
    };
    if desc { ordered.reverse() } else { ordered }
}

/// 关键词是否命中任一字段（大小写不敏感）；空关键词恒为真。
pub(crate) fn matches_query(query: &str, fields: &[impl AsRef<str>]) -> bool {
    if query.is_empty() {
        return true;
    }
    fields
        .iter()
        .any(|field| field.as_ref().to_lowercase().contains(query))
}

/// 列表行的操作目标：五种条目各自用能唯一定位自己的标识。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowTarget {
    Container { id: String, name: String },
    Image(String),
    Volume(String),
    Network(String),
    Compose(String),
}

/// 列表行动作：行内按钮与右键菜单共用一套语义，避免两处行为漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowAction {
    Overview,
    Logs,
    Terminal,
    Stats,
    Files,
    Start,
    Stop,
    Restart,
    Run,
    Rename,
    Remove,
    CopyId,
    CopyName,
}

impl RowAction {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Overview => "container.detail.overview",
            Self::Logs => "container.detail.logs",
            Self::Terminal => "container.detail.terminal",
            Self::Stats => "container.detail.stats",
            Self::Files => "container.detail.files",
            Self::Start => "container.action.start",
            Self::Stop => "container.action.stop",
            Self::Restart => "container.action.restart",
            Self::Run => "container.run.title",
            Self::Rename => "container.action.rename",
            Self::Remove => "common.delete",
            Self::CopyId => "container.row.copy_id",
            Self::CopyName => "container.row.copy_name",
        }
    }

    pub(crate) fn icon(self) -> gpui_kit::assets::IconName {
        use gpui_kit::assets::IconName;
        match self {
            Self::Overview => IconName::Info,
            Self::Logs => IconName::ScrollText,
            Self::Terminal => IconName::Terminal,
            Self::Stats => IconName::Gauge,
            Self::Files => IconName::FolderOpen,
            Self::Start => IconName::Play,
            Self::Stop => IconName::Square,
            Self::Restart => IconName::RotateCw,
            Self::Run => IconName::Play,
            Self::Rename => IconName::Pen,
            Self::Remove => IconName::Trash,
            Self::CopyId => IconName::Copy,
            Self::CopyName => IconName::Copy,
        }
    }

    /// 破坏性动作在菜单里要用危险色，与确认条保持一致。
    pub(crate) fn is_danger(self) -> bool {
        matches!(self, Self::Remove)
    }

    /// 该动作适用于哪类条目：右键菜单按目标类型生成，避免出现无意义项。
    pub(crate) fn applies_to(self, target: &RowTarget) -> bool {
        match self {
            Self::Logs | Self::Terminal | Self::Stats | Self::Files | Self::Restart => {
                matches!(target, RowTarget::Container { .. })
            }
            Self::Start | Self::Stop | Self::Rename => {
                matches!(target, RowTarget::Container { .. })
            }
            // 卷与编排项目没有独立 ID，名称就是唯一标识，不给重复的复制项。
            Self::CopyId => matches!(
                target,
                RowTarget::Container { .. } | RowTarget::Image(_) | RowTarget::Network(_)
            ),
            // 只有镜像能「跑起来」；卷与网络没有运行语义。
            Self::Run => matches!(target, RowTarget::Image(_)),
            Self::Overview | Self::Remove | Self::CopyName => true,
        }
    }
}

/// 行主键：与工作区 `selected` 字段的取法保持一致。
pub(crate) fn row_key(target: &RowTarget) -> &str {
    match target {
        RowTarget::Container { id, .. } | RowTarget::Image(id) | RowTarget::Network(id) => id,
        RowTarget::Volume(name) | RowTarget::Compose(name) => name,
    }
}

/// 多选集合的翻转：勾选则加入，取消则移除。
///
/// 抽成自由函数只是为了能被单测覆盖——它决定批量操作的条数是否正确，
/// 错一个就等于对用户少发或多发一次命令。
pub(crate) fn toggle_membership(set: &mut HashSet<String>, id: &str) {
    if !set.remove(id) {
        set.insert(id.to_string());
    }
}

/// 批量删除的武装状态：第一次点击只武装，第二次才真的下发。
///
/// 设计原理（Why）：批量删除一次影响 N 个容器，逐个弹确认条反而更糟；用
/// 「同一批勾选状态下必须连点两次」达到等价保护，勾选变化即自动解除武装。
pub(crate) fn arm_or_fire(armed: bool, count: usize) -> (bool, bool) {
    if armed {
        (false, count > 0)
    } else {
        (true, false)
    }
}

/// 批量动作：一次作用在多条勾选上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerBatchAction {
    Start,
    Stop,
    Remove,
}

impl ContainerBatchAction {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Start => "container.action.start",
            Self::Stop => "container.action.stop",
            Self::Remove => "common.delete",
        }
    }

    pub(crate) fn lifecycle(self) -> transom_core::ContainerLifecycleAction {
        match self {
            Self::Start => transom_core::ContainerLifecycleAction::Start,
            Self::Stop => transom_core::ContainerLifecycleAction::Stop,
            // 批量删除用 force：勾选本身就代表「这些我不要了」，
            // 停止中/僵死的容器用非 force 删除会整批失败。
            Self::Remove => transom_core::ContainerLifecycleAction::Remove { force: true },
        }
    }

    pub(crate) fn is_dangerous(self) -> bool {
        matches!(self, Self::Remove)
    }

    pub(crate) fn icon(self) -> gpui_kit::assets::IconName {
        match self {
            Self::Start => gpui_kit::assets::IconName::Play,
            Self::Stop => gpui_kit::assets::IconName::Square,
            Self::Remove => gpui_kit::assets::IconName::Trash,
        }
    }
}

/// 按行切分日志正文；`reversed` 为真时最新在前。
///
/// 设计原理（Why）：跟随日志时最想知道的是「刚刚发生了什么」，而纯文本块没有
/// 自动滚到底的能力，把顺序翻过来能让最新内容始终出现在视野里。空行保留，
/// 因为引擎输出本身可能含空行，丢掉会让日志对不上号。
pub(crate) fn log_lines(text: &str, reversed: bool) -> Vec<&str> {
    let mut lines: Vec<&str> = text.lines().collect();
    if reversed {
        lines.reverse();
    }
    lines
}

/// 快捷筛选是否放行该卷：`UnusedVolumes` 需要它没有被任何容器挂载。
pub(crate) fn volume_passes_filter(filter: ContainerQuickFilter, in_use: bool) -> bool {
    match filter {
        ContainerQuickFilter::UnusedVolumes => !in_use,
        _ => true,
    }
}

/// 快捷筛选是否放行该镜像：`DanglingImages` 需要它没有任何标签。
pub(crate) fn image_passes_filter(filter: ContainerQuickFilter, tags: &[String]) -> bool {
    match filter {
        ContainerQuickFilter::DanglingImages => tags.is_empty(),
        _ => true,
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
    pub(crate) _subscription: Subscription,
}

/// 文件浏览状态。
#[derive(Default)]
pub(crate) struct ContainerFilesState {
    pub(crate) path: String,
    pub(crate) entries: Vec<ContainerFileEntry>,
    pub(crate) loading: bool,
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
    /// 已按下一次「删除连接」、等待第二次确认的档案 ID。
    ///
    /// 设计原理（Why）：删除引擎连接会连带关掉用它打开的工作区标签，属于
    /// 不可逆动作；市场工具都要求二次确认，而这里只有一条待确认记录，
    /// 用弹条反而更啰嗦。
    pub(crate) pending_delete: Option<transom_core::EngineProfileId>,
    /// 侧栏当前单选高亮的引擎连接。
    pub(crate) sidebar_selected: Option<transom_core::EngineProfileId>,
    /// 侧栏搜索框：首次渲染时创建，随后复用。
    pub(crate) sidebar_search: Option<Entity<InputState>>,
    pub(crate) sidebar_search_subscription: Option<Subscription>,
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

/// 表单里的单行输入：占位符走 i18n，可选预填值。
fn run_input(
    placeholder: &'static str,
    default: Option<String>,
    window: &mut Window,
    cx: &mut Context<crate::app::AppShell>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let state = InputState::new(window, cx).placeholder(rust_i18n::t!(placeholder).to_string());
        match default {
            Some(value) => state.default_value(value),
            None => state,
        }
    })
}

/// 运行容器表单：把镜像跑起来的最小必要参数。
///
/// 设计原理（Why）：`docker run` 的参数面极大，界面只开放真正决定「能不能跑起来」
/// 的六项（名称、镜像、命令、端口、挂载、环境变量、重启策略），其余走镜像与引擎
/// 默认；文本框按 `docker run` 的常见写法收集，再解析成强类型 spec 交给引擎层校验，
/// 这样用户写 `8080:80/tcp` 与 `8080:80` 都能得到同一个映射。
pub(crate) struct ContainerRunForm {
    /// 表单归属的工作区标签。
    pub(crate) tab_id: TabId,
    pub(crate) name: Entity<InputState>,
    pub(crate) image: Entity<InputState>,
    pub(crate) command: Entity<InputState>,
    pub(crate) ports: Entity<InputState>,
    pub(crate) binds: Entity<InputState>,
    pub(crate) env: Entity<InputState>,
    pub(crate) health: Entity<InputState>,
    pub(crate) health_interval: Entity<InputState>,
    pub(crate) memory: Entity<InputState>,
    pub(crate) cpus: Entity<InputState>,
    pub(crate) restart: Option<String>,
    /// 校验或引擎返回的错误提示，展示在表单底部。
    pub(crate) error: Option<String>,
}

impl ContainerRunForm {
    pub(crate) fn new(
        tab_id: TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        Self {
            tab_id,
            name: run_input("container.run.name_placeholder", None, window, cx),
            image: run_input("container.run.image_placeholder", image_hint, window, cx),
            command: run_input("container.run.command_placeholder", None, window, cx),
            ports: run_input("container.run.ports_placeholder", None, window, cx),
            binds: run_input("container.run.binds_placeholder", None, window, cx),
            env: run_input("container.run.env_placeholder", None, window, cx),
            health: run_input("container.run.health_placeholder", None, window, cx),
            health_interval: run_input(
                "container.run.health_interval_placeholder",
                None,
                window,
                cx,
            ),
            memory: run_input("container.run.memory_placeholder", None, window, cx),
            cpus: run_input("container.run.cpus_placeholder", None, window, cx),
            restart: None,
            error: None,
        }
    }

    /// 读取输入并解析成 spec；解析与校验错误直接返回中文提示。
    ///
    /// # Errors
    /// 端口/挂载写法非法、spec 校验不通过时返回可直接展示的中文原因。
    pub(crate) fn to_spec(&self, cx: &App) -> Result<ContainerCreateSpec, String> {
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let mut spec = ContainerCreateSpec {
            name: read(&self.name),
            image: read(&self.image),
            command: split_argv(&read(&self.command)),
            env: split_list(&read(&self.env)),
            ports: parse_ports(&read(&self.ports))?,
            binds: split_list(&read(&self.binds)),
            restart: self.restart.clone(),
            health_cmd: Some(read(&self.health)),
            // 间隔留空或写非数字都按引擎默认处理：这不是必须填对的字段，
            // 报错打断提交流程不值得，真正的非法值（0 秒）由 spec 校验拦下。
            health_interval_secs: read(&self.health_interval).parse::<u64>().ok(),
            memory_mb: read(&self.memory).parse::<u64>().ok(),
            nano_cpus: parse_cores(&read(&self.cpus)),
        };
        spec.validate().map_err(|error| error.to_string())?;
        Ok(spec)
    }
}

/// 逗号分隔列表：卷名、环境变量等都用它，空项直接丢弃。
pub(crate) fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// 把「核数」输入换算成 Docker 的 `NanoCpus`（1e-9 核）。
///
/// 留空、非数字、非正数都按「不限制」处理：这一项不是必填，报错打断提交流程
/// 不值得，真正的量级错误由 spec 校验拦下。
pub(crate) fn parse_cores(text: &str) -> Option<u64> {
    let cores: f64 = text.trim().parse().ok()?;
    if !cores.is_finite() || cores <= 0.0 {
        return None;
    }
    Some((cores * 1_000_000_000.0).round() as u64)
}

/// 命令按空白切分为 argv；本工具不支持带引号的命令，避免把 Shell 语义搬进界面。
pub(crate) fn split_argv(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// 解析端口映射：`8080:80`、`8080:80/udp`、`127.0.0.1:8080:80` 之外的写法一律拒绝。
///
/// 返回的 `host` 为 0 或协议不认识都属于非法输入，错误信息直接可展示。
pub(crate) fn parse_ports(text: &str) -> Result<Vec<ContainerPortPublish>, String> {
    let mut items = Vec::new();
    for chunk in split_list(text) {
        let (mapping, protocol) = match chunk.split_once('/') {
            Some((mapping, protocol)) => (mapping, protocol.to_ascii_lowercase()),
            None => (chunk.as_str(), "tcp".to_string()),
        };
        let mut parts = mapping.split(':').collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(rust_i18n::t!("container.run.port_format", item = chunk).to_string());
        }
        let host = parts[0].parse::<u16>().map_err(|_| {
            rust_i18n::t!("container.run.port_number", value = parts[0].to_string()).to_string()
        })?;
        let container = parts[1].parse::<u16>().map_err(|_| {
            rust_i18n::t!("container.run.port_number", value = parts[1].to_string()).to_string()
        })?;
        parts.clear();
        items.push(ContainerPortPublish {
            host,
            container,
            protocol,
        });
    }
    Ok(items)
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

#[cfg(test)]
mod tests {
    use super::{
        ContainerQuickFilter, ContainerSection, ContainerSortKey, ContainerStateFilter, SortFields,
        compare_fields, matches_query,
    };

    fn fields(name: &str, state: &str, created: i64, size: u64) -> SortFields {
        SortFields {
            name: name.to_string(),
            state: state.to_string(),
            created,
            size,
        }
    }

    fn order(items: &[SortFields], key: ContainerSortKey, desc: bool) -> Vec<String> {
        let mut sorted = items.to_vec();
        sorted.sort_by(|left, right| compare_fields(left, right, key, desc));
        sorted.into_iter().map(|item| item.name).collect()
    }

    #[test]
    fn checkbox_membership_and_batch_arm_toggle() {
        let mut set = std::collections::HashSet::new();
        super::toggle_membership(&mut set, "a");
        assert_eq!(set.len(), 1, "首次点击加入");
        super::toggle_membership(&mut set, "a");
        assert!(set.is_empty(), "再次点击移除");
        super::toggle_membership(&mut set, "b");
        super::toggle_membership(&mut set, "c");
        assert_eq!(set.len(), 2);

        assert_eq!(
            super::arm_or_fire(false, 3),
            (true, false),
            "第一次只是武装"
        );
        assert_eq!(super::arm_or_fire(true, 3), (false, true), "第二次才下发");
        assert_eq!(
            super::arm_or_fire(true, 0),
            (false, false),
            "没有选中项就不下发"
        );
    }

    #[test]
    fn quick_filters_only_apply_where_they_mean_something() {
        assert_eq!(
            ContainerQuickFilter::for_section(ContainerSection::Volumes),
            [
                ContainerQuickFilter::All,
                ContainerQuickFilter::UnusedVolumes
            ]
        );
        assert_eq!(
            ContainerQuickFilter::for_section(ContainerSection::Containers),
            [ContainerQuickFilter::All]
        );
        assert!(super::volume_passes_filter(ContainerQuickFilter::All, true));
        assert!(!super::volume_passes_filter(
            ContainerQuickFilter::UnusedVolumes,
            true
        ));
        assert!(super::volume_passes_filter(
            ContainerQuickFilter::UnusedVolumes,
            false
        ));
        assert!(super::image_passes_filter(
            ContainerQuickFilter::All,
            &["nginx".to_string()]
        ));
        assert!(!super::image_passes_filter(
            ContainerQuickFilter::DanglingImages,
            &["nginx".to_string()]
        ));
        assert!(super::image_passes_filter(
            ContainerQuickFilter::DanglingImages,
            &[]
        ));
    }

    #[test]
    fn log_lines_reverse_keeps_every_line() {
        let text = "a\nb\n\nc";
        assert_eq!(super::log_lines(text, false), vec!["a", "b", "", "c"]);
        assert_eq!(super::log_lines(text, true), vec!["c", "", "b", "a"]);
        assert!(super::log_lines("", false).is_empty());
    }

    #[test]
    fn state_filter_covers_both_states() {
        assert!(ContainerStateFilter::All.accepts(true));
        assert!(ContainerStateFilter::All.accepts(false));
        assert!(ContainerStateFilter::Running.accepts(true));
        assert!(!ContainerStateFilter::Running.accepts(false));
        assert!(ContainerStateFilter::Stopped.accepts(false));
        assert!(!ContainerStateFilter::Stopped.accepts(true));
    }

    #[test]
    fn sorting_is_stable_across_equal_keys() {
        let items = [
            fields("web", "running", 30, 10),
            fields("api", "running", 10, 30),
            fields("db", "exited", 20, 20),
        ];
        assert_eq!(
            order(&items, ContainerSortKey::Name, false),
            ["api", "db", "web"]
        );
        assert_eq!(
            order(&items, ContainerSortKey::Name, true),
            ["web", "db", "api"]
        );
        assert_eq!(
            order(&items, ContainerSortKey::State, false),
            ["db", "api", "web"],
            "exited 排在 running 之前，同状态按名称收口"
        );
        assert_eq!(
            order(&items, ContainerSortKey::Created, false),
            ["api", "db", "web"]
        );
        assert_eq!(
            order(&items, ContainerSortKey::Size, true),
            ["api", "db", "web"]
        );
    }

    #[test]
    fn each_section_only_offers_valid_sort_keys() {
        assert!(
            !ContainerSortKey::for_section(ContainerSection::Volumes)
                .contains(&ContainerSortKey::Size)
        );
        assert!(
            !ContainerSortKey::for_section(ContainerSection::Images)
                .contains(&ContainerSortKey::State)
        );
        assert!(
            ContainerSortKey::for_section(ContainerSection::Networks) == [ContainerSortKey::Name]
        );
        for section in ContainerSection::ALL {
            let keys = ContainerSortKey::for_section(section);
            assert!(!keys.is_empty(), "{section:?} 必须有可用排序键");
            assert!(keys.contains(&ContainerSortKey::default_for(section)));
        }
    }

    #[test]
    fn cpu_cores_convert_to_nanocpus() {
        assert_eq!(super::parse_cores("0.5"), Some(500_000_000));
        assert_eq!(super::parse_cores(" 2 "), Some(2_000_000_000));
        assert_eq!(super::parse_cores(""), None);
        assert_eq!(super::parse_cores("0"), None, "0 表示不限制");
        assert_eq!(super::parse_cores("-1"), None);
        assert_eq!(super::parse_cores("abc"), None);
        assert_eq!(super::parse_cores("NaN"), None);
    }

    #[test]
    fn port_and_list_parsing_follows_run_flags() {
        let ports = super::parse_ports("8080:80, 53:53/udp ,").expect("合法端口映射");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].host, 8080);
        assert_eq!(ports[0].protocol, "tcp");
        assert_eq!(ports[1].container, 53);
        assert_eq!(ports[1].protocol, "udp");
        assert!(super::parse_ports("8080-80").is_err(), "缺分隔符必须报错");
        assert!(super::parse_ports("abc:80").is_err(), "非数字端口必须报错");
        assert_eq!(super::split_list("A=1,,B=2"), vec!["A=1", "B=2"]);
        assert_eq!(
            super::split_argv("  nginx   -g  daemon"),
            vec!["nginx", "-g", "daemon"]
        );
        assert!(super::split_argv("").is_empty());
    }

    #[test]
    fn query_matches_any_field_case_insensitively() {
        assert!(matches_query(
            "nginx",
            &["Web".to_string(), "nginx:latest".to_string()]
        ));
        assert!(!matches_query(
            "redis",
            &["Web".to_string(), "nginx:latest".to_string()]
        ));
        assert!(matches_query("", &["anything".to_string()]));
        assert!(!matches_query("x", &[] as &[String]));
    }
}
