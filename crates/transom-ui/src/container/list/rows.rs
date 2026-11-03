//! 列表行数据与行内动作：按分类收集、排序并生成动作集合。

use super::*;
use std::collections::HashSet;

/// 按分类收集行数据，并套用状态筛选与关键词搜索。
pub(crate) fn collect_rows(
    section: ContainerSection,
    workspace: &ContainerWorkspaceState,
    query: &str,
    filter: ContainerStateFilter,
) -> Vec<ListRow> {
    let quick = workspace.quick_filter;
    let cli_backend = workspace.info.as_ref().is_some_and(|info| info.cli_backend);
    // CLI 兜底的容器摘要没有挂载明细，算不出在用卷，此时既不显示徽章也不给筛选。
    let usage_known = !cli_backend && !workspace.containers.is_empty();
    // 在用卷集合由已拉到的容器挂载计算：列表接口就带 Mounts，不需要逐个 inspect。
    let used_volumes: HashSet<String> = if usage_known {
        workspace
            .containers
            .iter()
            .flat_map(|item| item.mounts.iter())
            .filter(|mount| mount.kind == "volume")
            .map(|mount| mount.source.clone())
            .collect()
    } else {
        HashSet::default()
    };
    match section {
        ContainerSection::Containers => workspace
            .containers
            .iter()
            .filter(|item| filter.accepts(item.state.is_running()))
            .filter(|item| {
                matches_query(
                    query,
                    &[
                        item.name.clone(),
                        item.image.clone(),
                        item.state.key().to_string(),
                        item.compose_project.clone().unwrap_or_default(),
                    ],
                )
            })
            .map(container_row_spec)
            .collect(),
        ContainerSection::Images => workspace
            .images
            .iter()
            .filter(|item| matches_query(query, &[item.display_name(), short_id(&item.id)]))
            .filter(|item| image_passes_filter(quick, &item.tags))
            .map(image_row_spec)
            .collect(),
        ContainerSection::Volumes => workspace
            .volumes
            .iter()
            .filter(|item| matches_query(query, &[item.name.clone(), item.driver.clone()]))
            .filter(|item| volume_passes_filter(quick, used_volumes.contains(&item.name)))
            .map(|item| volume_row_spec(item, usage_known, used_volumes.contains(&item.name)))
            .collect(),
        ContainerSection::Networks => workspace
            .networks
            .iter()
            .filter(|item| {
                matches_query(
                    query,
                    &[
                        item.name.clone(),
                        item.driver.clone(),
                        item.subnet.clone().unwrap_or_default(),
                    ],
                )
            })
            .map(network_row_spec)
            .collect(),
        ContainerSection::Compose => workspace
            .compose
            .iter()
            .filter(|item| {
                matches_query(
                    query,
                    &[
                        item.name.clone(),
                        item.working_dir.clone().unwrap_or_default(),
                    ],
                )
            })
            .map(compose_row_spec)
            .collect(),
    }
}

pub(crate) fn container_row_spec(item: &ContainerSummary) -> ListRow {
    ListRow {
        key: format!("container-row-{}", item.id),
        title: item.name.clone(),
        // 单行只剩名称：镜像引用（走加速仓库时三四十字符）与状态串在这一列宽度
        // 下必然截断成不可读片段，完整信息在右侧详情基础信息里看。
        subtitle: None,
        badge: None,
        running: Some(item.state.is_running()),
        target: RowTarget::Container {
            id: item.id.clone(),
            name: item.name.clone(),
        },
        fields: SortFields {
            name: item.name.clone(),
            state: item.state.key().to_string(),
            created: item.created,
            size: 0,
        },
    }
}

pub(crate) fn image_row_spec(item: &ContainerImageSummary) -> ListRow {
    ListRow {
        key: format!("image-row-{}", item.id),
        // 短名做主标题：完整引用（含加速仓库主机）留在详情与起容器参数里。
        title: item.short_display_name(),
        subtitle: Some(format!(
            "{} · {}",
            format_size(item.size),
            short_id(&item.id)
        )),
        badge: None,
        running: None,
        target: RowTarget::Image(item.id.clone()),
        fields: SortFields {
            name: item.display_name(),
            state: String::new(),
            created: item.created,
            size: item.size,
        },
    }
}

pub(crate) fn volume_row_spec(
    item: &ContainerVolumeSummary,
    usage_known: bool,
    in_use: bool,
) -> ListRow {
    ListRow {
        key: format!("volume-row-{}", item.name),
        title: item.name.clone(),
        subtitle: Some(format!("{} · {}", item.driver, item.mountpoint)),
        // 在用/空闲改用名称前的状态点，与容器行同一套语言；文字徽章去掉，
        // 避免同一件事在一行里说两遍。用量数据拿不到时不画点（不假装有把握）。
        badge: None,
        running: usage_known.then_some(in_use),
        target: RowTarget::Volume(item.name.clone()),
        fields: SortFields {
            name: item.name.clone(),
            state: String::new(),
            created: parse_created(item.created.as_deref()),
            size: 0,
        },
    }
}

pub(crate) fn network_row_spec(item: &ContainerNetworkSummary) -> ListRow {
    ListRow {
        key: format!("network-row-{}", item.id),
        title: item.name.clone(),
        subtitle: Some(format!(
            "{} · {}{}",
            item.driver,
            item.scope,
            item.subnet
                .as_ref()
                .map(|subnet| format!(" · {subnet}"))
                .unwrap_or_default()
        )),
        badge: None,
        running: None,
        target: RowTarget::Network(item.id.clone()),
        fields: SortFields {
            name: item.name.clone(),
            state: String::new(),
            created: 0,
            size: 0,
        },
    }
}

pub(crate) fn compose_row_spec(item: &ComposeProject) -> ListRow {
    ListRow {
        key: format!("compose-row-{}", item.name),
        title: item.name.clone(),
        subtitle: Some(format!(
            "{} · {}",
            item.state_label(),
            item.working_dir.clone().unwrap_or_default()
        )),
        badge: None,
        running: Some(item.total > 0 && item.running == item.total),
        target: RowTarget::Compose(item.name.clone()),
        fields: SortFields {
            name: item.name.clone(),
            state: item.state_label().to_string(),
            created: 0,
            size: item.total as u64,
        },
    }
}

/// 卷的创建时间是 RFC3339 字符串；解析失败按 0 处理，只影响排序不影响展示。
pub(crate) fn parse_created(value: Option<&str>) -> i64 {
    value
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or_default()
}

/// 该行的完整动作清单（右键菜单），顺序即菜单顺序。
pub(crate) fn row_actions(row: &ListRow) -> Vec<RowAction> {
    let mut actions = vec![RowAction::Overview];
    if matches!(row.target, RowTarget::Container { .. }) {
        actions.extend([
            RowAction::Logs,
            RowAction::Terminal,
            RowAction::Stats,
            RowAction::Files,
        ]);
        actions.push(if row.running == Some(true) {
            RowAction::Pause
        } else {
            RowAction::Unpause
        });
        actions.push(if row.running == Some(true) {
            RowAction::Stop
        } else {
            RowAction::Start
        });
        actions.push(RowAction::Restart);
        if row.running == Some(true) {
            actions.push(RowAction::Kill);
        }
        actions.push(RowAction::Rename);
    }
    if matches!(row.target, RowTarget::Image(_)) {
        actions.push(RowAction::Run);
        actions.push(RowAction::Tag);
    }
    if matches!(row.target, RowTarget::Compose(_)) {
        actions.push(RowAction::EditComposeYaml);
    }
    actions.push(RowAction::CopyName);
    actions.push(RowAction::CopyId);
    actions.push(RowAction::Remove);
    actions
        .into_iter()
        .filter(|action| action.applies_to(&row.target))
        .collect()
}

/// 行内快捷按钮：容器给「启停 + 重启 + 删除」，镜像给「运行 + 删除」，卷与网络给「删除」。
///
/// 设计原理（Why）：这一列是用户扫一眼就要动手的地方，最高频的启停/删除不该只
/// 藏在详情头部或右键菜单里（真实窗口实测：只有名称的行要点完再找按钮，来回移动
/// 鼠标成本高）。其余动作（日志、终端、统计、复制）仍然只在头部与菜单，避免同一
/// 件事摆三个入口。
pub(crate) fn quick_actions(row: &ListRow) -> Vec<RowAction> {
    let actions: Vec<RowAction> = match row.target {
        RowTarget::Container { .. } => vec![
            if row.running == Some(true) {
                RowAction::Stop
            } else {
                RowAction::Start
            },
            RowAction::Restart,
            RowAction::Remove,
        ],
        RowTarget::Image(_) => vec![RowAction::Run, RowAction::Remove],
        RowTarget::Volume(_) | RowTarget::Network(_) => vec![RowAction::Remove],
        RowTarget::Compose(_) => vec![RowAction::EditComposeYaml],
    };
    actions
        .into_iter()
        .filter(|action| action.applies_to(&row.target))
        .take(QUICK_LIMIT)
        .collect()
}
