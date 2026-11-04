//! 分类、页签、筛选与排序：纯枚举与谓词，不持有界面状态。

/// 工作区顶部功能标签。
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
            Self::Compose => IconName::Blocks,
            Self::Containers => IconName::Boxes,
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
                ContainerDetailTab::Changes,
            ],
            Self::Compose => &[ContainerDetailTab::Overview, ContainerDetailTab::Logs],
            Self::Images => &[ContainerDetailTab::Overview, ContainerDetailTab::History],
            Self::Volumes => &[ContainerDetailTab::Overview, ContainerDetailTab::Files],
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
    Changes,
    History,
}

impl ContainerDetailTab {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Overview => "container.detail.overview",
            Self::Logs => "container.detail.logs",
            Self::Terminal => "container.detail.terminal",
            Self::Stats => "container.detail.stats",
            Self::Files => "container.detail.files",
            Self::Changes => "container.detail.changes",
            Self::History => "container.detail.history",
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
