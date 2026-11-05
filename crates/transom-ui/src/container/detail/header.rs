//! 详情区头部：标题、状态与页签动作。

use super::*;
use crate::container::CONTENT_HEADER_HEIGHT;

/// 详情头部的渲染参数：选中项按分类不同，统一用借用结构体传入。
pub(crate) struct ContainerHeaderView<'a> {
    pub(super) section: ContainerSection,
    pub(super) title: String,
    /// 详情页签按钮组：容器分类有页签，直接占掉标题的位置（标题与中栏高亮行
    /// 重复）；其它分类只有基础信息，页签为 `None`，仍显示标题。
    pub(super) tabs: Option<AnyElement>,
    /// 基础信息当前是否显示原始 inspect：头部的小按钮用它显示选中态。
    pub(super) overview_raw: bool,
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

impl AppShell {
    pub(crate) fn render_detail_header(
        &mut self,
        tab_id: TabId,
        view: ContainerHeaderView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ContainerHeaderView {
            section,
            title,
            tabs,
            overview_raw,
            detail_tab,
            container,
            details,
            image,
            volume,
            network,
            compose,
            all_containers,
            timeline,
        } = view;
        // 健康状态点：healthy 绿、unhealthy 红、starting 等其他状态弱色。
        // 只在镜像声明了 HEALTHCHECK 时出现，没声明就不制造噪声。
        let health_chip: Option<AnyElement> = details
            .as_ref()
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
            });
        let mut actions = h_flex().gap_1().items_center().flex_shrink_0();
        match section {
            ContainerSection::Containers => {
                if let Some(container) = container {
                    actions = actions.children(super::header_container::container_header_actions(
                        tab_id,
                        container,
                        detail_tab,
                        overview_raw,
                        cx,
                    ));
                }
            }
            ContainerSection::Compose => {
                if let Some(project) = compose {
                    actions = actions.children(super::header_actions::compose_header_actions(
                        tab_id,
                        project,
                        all_containers,
                        cx,
                    ));
                }
            }
            ContainerSection::Images => {
                if let Some(image) = image {
                    actions = actions.children(super::header_actions::image_header_actions(
                        tab_id, image, cx,
                    ));
                }
            }
            ContainerSection::Volumes => {
                if let Some(volume) = volume {
                    let vol_name_backup = volume.name.clone();
                    actions = actions.child(
                        Button::new("volume-backup")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Download)
                            .tooltip(rust_i18n::t!("container.action.backup_volume").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.backup_volume(tab_id, vol_name_backup.clone(), cx);
                            })),
                    );
                    let vol_name_restore = volume.name.clone();
                    actions = actions.child(
                        Button::new("volume-restore")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Upload)
                            .tooltip(rust_i18n::t!("container.action.restore_volume").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.restore_volume(tab_id, Some(vol_name_restore.clone()), cx);
                            })),
                    );
                    actions = actions.child(delete_button(
                        "volume-delete",
                        cx,
                        tab_id,
                        ContainerConfirm::Volume(volume.name.clone()),
                    ));
                }
            }
            ContainerSection::Networks => {
                if let Some(network) = network {
                    actions = actions.children(super::header_actions::network_header_actions(
                        tab_id, network, cx,
                    ));
                }
            }
        }

        // 头部左侧：根据规范显示「容器名 15px/600 + 状态胶囊 + 健康点」并跟随页签组
        let left: AnyElement = match section {
            ContainerSection::Containers => {
                let mut title_row = h_flex().gap_2().items_center().min_w_0();
                title_row = title_row.child(
                    Icon::new(section.icon())
                        .small()
                        .text_color(p.primary),
                );
                title_row = title_row.child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .max_w(px(220.0))
                        .truncate()
                        .child(title),
                );

                if let Some(c) = container {
                    let (state_label, state_color) = match c.state {
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
                            rust_i18n::t!(format!("container.state.{}", c.state.key())).to_string(),
                            p.muted_foreground,
                        ),
                    };
                    title_row = title_row.child(crate::widgets::status_badge(state_label, state_color, p));
                }

                if let Some(chip) = health_chip {
                    title_row = title_row.child(chip);
                }

                if let Some(tl) = timeline {
                    title_row = title_row.child(super::timeline::render_status_timeline(&tl, p));
                }

                if let Some(tabs) = tabs {
                    title_row = title_row
                        .child(div().w(px(1.0)).h(px(16.0)).bg(p.divider(p.border)).mx_1())
                        .child(tabs);
                }

                title_row.into_any_element()
            }
            _ => {
                let mut row = h_flex().gap_2().items_center().min_w_0();
                row = row
                    .child(Icon::new(section.icon()).small().text_color(p.primary))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .max_w(px(260.0))
                            .truncate()
                            .child(title),
                    );
                if let Some(tabs) = tabs {
                    row = row
                        .child(div().w(px(1.0)).h(px(16.0)).bg(p.divider(p.border)).mx_1())
                        .child(tabs);
                }
                row.into_any_element()
            }
        };

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
            .child(left)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_shrink_0()
                    .child(actions),
            )
            .into_any_element()
    }
}

pub(crate) fn detail_icon(tab: ContainerDetailTab) -> IconName {
    match tab {
        ContainerDetailTab::Overview => IconName::Info,
        ContainerDetailTab::Logs => IconName::ScrollText,
        ContainerDetailTab::Terminal => IconName::Terminal,
        ContainerDetailTab::Stats => IconName::Gauge,
        ContainerDetailTab::Files => IconName::FolderOpen,
        ContainerDetailTab::Changes => IconName::FileDiff,
        ContainerDetailTab::History => IconName::Clock,
        ContainerDetailTab::Yaml => IconName::FileText,
    }
}
