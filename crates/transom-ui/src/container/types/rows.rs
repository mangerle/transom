//! 列表行标识、行内动作与批量动作。

use super::*;

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
    Pause,
    Unpause,
    Kill,
    Run,
    Tag,
    Rename,
    EditComposeYaml,
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
            Self::Pause => "container.action.pause",
            Self::Unpause => "container.action.unpause",
            Self::Kill => "container.action.kill",
            Self::Run => "container.run.title",
            Self::Tag => "container.action.tag",
            Self::Rename => "container.action.rename",
            Self::EditComposeYaml => "container.compose.edit_yaml",
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
            Self::Pause => IconName::Pause,
            Self::Unpause => IconName::Play,
            Self::Kill => IconName::CircleAlert,
            Self::Run => IconName::Play,
            Self::Tag => IconName::TagPlus,
            Self::Rename => IconName::Pen,
            Self::EditComposeYaml => IconName::FilePen,
            Self::Remove => IconName::Trash,
            Self::CopyId => IconName::Copy,
            Self::CopyName => IconName::Copy,
        }
    }

    /// 破坏性动作在菜单里要用危险色，与确认条保持一致。
    pub(crate) fn is_danger(self) -> bool {
        matches!(self, Self::Remove | Self::Kill)
    }

    /// 该动作适用于哪类条目：右键菜单按目标类型生成，避免出现无意义项。
    pub(crate) fn applies_to(self, target: &RowTarget) -> bool {
        match self {
            Self::Logs | Self::Terminal | Self::Stats | Self::Files | Self::Restart => {
                matches!(target, RowTarget::Container { .. })
            }
            Self::Start | Self::Stop | Self::Pause | Self::Unpause | Self::Kill | Self::Rename => {
                matches!(target, RowTarget::Container { .. })
            }
            Self::EditComposeYaml => matches!(target, RowTarget::Compose(_)),
            // 卷与编排项目没有独立 ID，名称就是唯一标识，不给重复的复制项。
            Self::CopyId => matches!(
                target,
                RowTarget::Container { .. } | RowTarget::Image(_) | RowTarget::Network(_)
            ),
            // 只有镜像能「跑起来」与打标签；卷与网络没有运行语义。
            Self::Run | Self::Tag => matches!(target, RowTarget::Image(_)),
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
