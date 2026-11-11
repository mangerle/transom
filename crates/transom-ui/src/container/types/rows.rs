//! 列表行标识、行内动作与批量动作。

use super::*;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

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
    Push,
    Save,
    History,
    Rename,
    ExportConfig,
    Commit,
    Clone,
    EditComposeYaml,
    BackupVolume,
    RestoreVolume,
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
            Self::Push => "container.action.push",
            Self::Save => "container.action.save_image",
            Self::History => "container.detail.history",
            Self::Rename => "container.action.rename",
            Self::ExportConfig => "container.action.export_config",
            Self::Commit => "container.action.commit",
            Self::Clone => "container.action.clone",
            Self::EditComposeYaml => "container.compose.edit_yaml",
            Self::BackupVolume => "container.action.backup_volume",
            Self::RestoreVolume => "container.action.restore_volume",
            Self::Remove => "common.delete",
            Self::CopyId => "container.row.copy_id",
            Self::CopyName => "container.row.copy_name",
        }
    }

    pub(crate) fn icon(self) -> transom_ui_icons::AppIcon {
        use transom_ui_icons::AppIcon;
        match self {
            Self::Overview => AppIcon::Custom(CustomIcon::INFO),
            Self::Logs => AppIcon::Ui(UiIcon::LIST_FILES),
            Self::Terminal => AppIcon::Domain(DomainIcon::TERMINAL),
            Self::Stats => AppIcon::Custom(CustomIcon::PROCESS),
            Self::Files => AppIcon::Ui(UiIcon::FOLDER),
            Self::Start => AppIcon::Ui(UiIcon::RUN),
            Self::Stop => AppIcon::Ui(UiIcon::STOP),
            Self::Restart => AppIcon::Ui(UiIcon::REFRESH),
            Self::Pause => AppIcon::Ui(UiIcon::PAUSE),
            Self::Unpause => AppIcon::Ui(UiIcon::RUN),
            Self::Kill => AppIcon::Ui(UiIcon::ERROR),
            Self::Run => AppIcon::Ui(UiIcon::RUN),
            Self::Tag => AppIcon::Ui(UiIcon::TAG),
            Self::Push => AppIcon::Ui(UiIcon::UPLOAD),
            Self::Save => AppIcon::Ui(UiIcon::DOWNLOAD),
            Self::History => AppIcon::Ui(UiIcon::HISTORY),
            Self::Rename => AppIcon::Ui(UiIcon::EDIT),
            Self::ExportConfig => AppIcon::Ui(UiIcon::DIFF),
            Self::Commit => AppIcon::Custom(CustomIcon::IMAGE),
            Self::Clone => AppIcon::Ui(UiIcon::COPY),
            Self::EditComposeYaml => AppIcon::Ui(UiIcon::EDIT),
            Self::BackupVolume => AppIcon::Ui(UiIcon::DOWNLOAD),
            Self::RestoreVolume => AppIcon::Ui(UiIcon::UPLOAD),
            Self::Remove => AppIcon::Ui(UiIcon::DELETE),
            Self::CopyId => AppIcon::Ui(UiIcon::COPY),
            Self::CopyName => AppIcon::Ui(UiIcon::COPY),
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
            Self::Start
            | Self::Stop
            | Self::Pause
            | Self::Unpause
            | Self::Kill
            | Self::Rename
            | Self::ExportConfig
            | Self::Commit
            | Self::Clone => {
                matches!(target, RowTarget::Container { .. })
            }
            Self::EditComposeYaml => matches!(target, RowTarget::Compose(_)),
            Self::BackupVolume | Self::RestoreVolume => matches!(target, RowTarget::Volume(_)),
            // 卷与编排项目没有独立 ID，名称就是唯一标识，不给重复的复制项。
            Self::CopyId => matches!(
                target,
                RowTarget::Container { .. } | RowTarget::Image(_) | RowTarget::Network(_)
            ),
            // 只有镜像能「跑起来」、打标签、推送、导出与查看构建历史；卷与网络没有对应语义。
            Self::Run | Self::Tag | Self::Push | Self::Save | Self::History => {
                matches!(target, RowTarget::Image(_))
            }
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

/// 按关键字过滤日志行；只保留最后 `max_lines` 行，`reversed` 为真时最新在前。
///
/// 设计原理（Why）：跟随日志时界面每帧只需要尾部窗口；从末尾反向取够行数再做
/// 过滤，既避免全量收集，也避免对上万行逐一构建 UI 元素。
pub(crate) fn filtered_log_lines<'a>(
    text: &'a str,
    filter: &str,
    reversed: bool,
    max_lines: usize,
) -> Vec<&'a str> {
    if max_lines == 0 {
        return Vec::new();
    }
    let filter = filter.trim();
    let filter_lower = filter.to_lowercase();
    let mut lines: Vec<&'a str> = text
        .lines()
        .rev()
        .take(max_lines)
        .filter(|line| filter.is_empty() || line.to_lowercase().contains(&filter_lower))
        .collect();
    if !reversed {
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
