//! 详情区头部：标题、状态与页签动作。

use super::*;
use crate::container::{CONTENT_HEADER_HEIGHT, ContainerWorkspace, ContainerWorkspaceEvent};
use crate::theme::tokens::{control, typography};
use crate::widgets::{IconButton, toolbar_separator};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 详情头部的渲染参数：选中项按分类不同，统一用借用结构体传入。
pub(crate) struct ContainerHeaderView<'a> {
    /// 工作区实体句柄：动作按钮直接更新实体或上报外壳意图。
    pub(super) view: &'a Entity<ContainerWorkspace>,
    pub(super) section: ContainerSection,
    pub(super) title: String,
    /// 详情页签按钮组：容器分类有页签，直接占掉标题的位置（标题与中栏高亮行
    /// 重复）；其它分类只有基础信息，页签为 `None`，仍显示标题。
    pub(super) tabs: Option<AnyElement>,
    /// 当前详情页签：原始 inspect 按钮只在基础信息页出现。
    pub(super) detail_tab: ContainerDetailTab,
    pub(super) container: &'a Option<ContainerSummary>,
    /// 容器详情：健康状态只有 inspect 才有，标题行要显示它。
    pub(super) details: &'a Option<ContainerDetails>,
    pub(super) image: &'a Option<transom_core::ContainerImageSummary>,
    pub(super) volume: &'a Option<transom_core::ContainerVolumeSummary>,
    pub(super) network: &'a Option<transom_core::ContainerNetworkSummary>,
    pub(super) compose: &'a Option<ComposeProject>,
    pub(super) all_containers: &'a [ContainerSummary],
    /// 容器 24 小时状态时间线。
    pub(super) timeline: Option<super::timeline::StatusTimeline>,
}

pub(super) fn render_detail_header(mut params: ContainerHeaderView<'_>, p: &Palette) -> AnyElement {
    let tabs = params.tabs.take();
    let health_chip = build_health_chip(params.details.as_ref(), p);
    let actions = build_header_actions(&params);
    let left = build_header_left(&params, tabs, health_chip, p);

    h_flex()
        .h(rems(CONTENT_HEADER_HEIGHT))
        .px_3()
        .gap_3()
        .items_center()
        .justify_between()
        .flex_none()
        .min_w_0()
        .overflow_hidden()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(div().flex_1().min_w_0().overflow_hidden().child(left))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .flex_shrink_0()
                .child(actions),
        )
        .into_any_element()
}

/// 构造健康检查状态点；仅在镜像声明了 HEALTHCHECK 时显示。
fn build_health_chip(
    details: Option<&transom_core::ContainerDetails>,
    p: &Palette,
) -> Option<AnyElement> {
    details
        .and_then(|details| details.health.clone())
        .map(|health| {
            let color = if health == "healthy" {
                p.success
            } else if health == "unhealthy" {
                p.danger
            } else {
                p.muted_foreground
            };
            h_flex()
                .gap_1()
                .items_center()
                .flex_none()
                .child(div().size_2().rounded_full().bg(color))
                .child(div().text_xs().text_color(p.muted_foreground).child(health))
                .into_any_element()
        })
}

/// 构造头部右侧各区域的操作按钮栏。
fn build_header_actions(params: &ContainerHeaderView<'_>) -> AnyElement {
    let view = params.view;
    let mut actions = h_flex().gap_1().items_center().flex_shrink_0();
    match params.section {
        ContainerSection::Containers => {
            if let Some(container) = params.container {
                actions = actions.children(super::header_container::container_header_actions(
                    view,
                    container,
                    params.detail_tab,
                ));
            }
        }
        ContainerSection::Compose => {
            if let Some(project) = params.compose {
                actions = actions.children(super::header_actions::compose_header_actions(
                    view,
                    project,
                    params.all_containers,
                ));
            }
        }
        ContainerSection::Images => {
            if let Some(image) = params.image {
                actions =
                    actions.children(super::header_actions::image_header_actions(view, image));
            }
        }
        ContainerSection::Volumes => {
            if let Some(volume) = params.volume {
                actions = actions.children(build_volume_header_actions(view, &volume.name));
            }
        }
        ContainerSection::Networks => {
            if let Some(network) = params.network {
                actions =
                    actions.children(super::header_actions::network_header_actions(view, network));
            }
        }
    }
    actions.into_any_element()
}

fn build_volume_header_actions(
    view: &Entity<ContainerWorkspace>,
    volume_name: &str,
) -> Vec<AnyElement> {
    let backup_view = view.clone();
    let vol_name_backup = volume_name.to_string();
    let restore_view = view.clone();
    let vol_name_restore = volume_name.to_string();
    vec![
        IconButton::new(
            "volume-backup",
            UiIcon::DOWNLOAD,
            rust_i18n::t!("container.action.backup_volume").to_string(),
        )
        .render()
        .on_click(move |_, _, cx| {
            backup_view.update(cx, |_, cx| {
                cx.emit(ContainerWorkspaceEvent::BackupVolume(
                    vol_name_backup.clone(),
                ));
            });
        })
        .into_any_element(),
        IconButton::new(
            "volume-restore",
            UiIcon::UPLOAD,
            rust_i18n::t!("container.action.restore_volume").to_string(),
        )
        .render()
        .on_click(move |_, _, cx| {
            restore_view.update(cx, |_, cx| {
                cx.emit(ContainerWorkspaceEvent::RestoreVolume(Some(
                    vol_name_restore.clone(),
                )));
            });
        })
        .into_any_element(),
        delete_button(
            view,
            "volume-delete",
            ContainerConfirm::Volume(volume_name.to_string()),
        ),
    ]
}

/// 构造头部左侧标题、状态胶囊与详情页签组。
fn build_header_left(
    params: &ContainerHeaderView<'_>,
    tabs: Option<AnyElement>,
    health_chip: Option<AnyElement>,
    p: &Palette,
) -> AnyElement {
    let max_title_w = if params.section == ContainerSection::Containers {
        px(220.0)
    } else {
        px(260.0)
    };
    let mut row = h_flex()
        .gap_2()
        .items_center()
        .min_w_0()
        .child(
            AppIconElement::new(params.section.icon())
                .with_rems(rems(control::ICON))
                .text_color(p.primary),
        )
        .child(
            div()
                .text_size(rems(typography::TITLE))
                .font_weight(FontWeight::SEMIBOLD)
                .max_w(max_title_w)
                .truncate()
                .child(params.title.clone()),
        );

    if params.section == ContainerSection::Containers {
        if let Some(container) = params.container {
            let (state_label, state_color) = container_state_badge(container.state, p);
            row = row.child(crate::widgets::status_badge(state_label, state_color, p));
        }
        if let Some(chip) = health_chip {
            row = row.child(chip);
        }
        if let Some(tl) = &params.timeline {
            row = row.child(super::timeline::render_status_timeline(tl, p));
        }
    }

    if let Some(tabs) = tabs {
        row = row.child(toolbar_separator(p).mx_1()).child(tabs);
    }
    row.into_any_element()
}

fn container_state_badge(state: transom_core::ContainerState, p: &Palette) -> (String, Hsla) {
    match state {
        transom_core::ContainerState::Running => (
            rust_i18n::t!("container.state.running").to_string(),
            p.success,
        ),
        transom_core::ContainerState::Paused => (
            rust_i18n::t!("container.state.paused").to_string(),
            p.warning,
        ),
        transom_core::ContainerState::Restarting => (
            rust_i18n::t!("container.state.restarting").to_string(),
            p.primary,
        ),
        transom_core::ContainerState::Exited => (
            rust_i18n::t!("container.state.exited").to_string(),
            p.muted_foreground,
        ),
        _ => (
            rust_i18n::t!(format!("container.state.{}", state.key())).to_string(),
            p.muted_foreground,
        ),
    }
}

pub(crate) fn detail_icon(tab: ContainerDetailTab) -> AppIcon {
    match tab {
        ContainerDetailTab::Overview => AppIcon::Custom(CustomIcon::INFO),
        ContainerDetailTab::Logs => AppIcon::Ui(UiIcon::LIST_FILES),
        ContainerDetailTab::Terminal => AppIcon::Domain(DomainIcon::TERMINAL),
        ContainerDetailTab::Stats => AppIcon::Custom(CustomIcon::PROCESS),
        ContainerDetailTab::Files => AppIcon::Ui(UiIcon::FOLDER),
        ContainerDetailTab::Changes => AppIcon::Ui(UiIcon::DIFF),
        ContainerDetailTab::History => AppIcon::Ui(UiIcon::HISTORY),
        ContainerDetailTab::Yaml => AppIcon::Ui(UiIcon::DIFF),
    }
}
