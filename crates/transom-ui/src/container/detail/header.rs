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
                    // 原始 inspect 只是一个视图切换，用一个小图标按钮就够；它只属于
                    // 基础信息页，切到日志/终端等页签时不该占位置。
                    if detail_tab == ContainerDetailTab::Overview {
                        let raw = overview_raw;
                        actions = actions.child(
                            Button::new("container-raw-inspect")
                                .xsmall()
                                .ghost()
                                .icon(IconName::Braces)
                                .selected(raw)
                                .tooltip(
                                    rust_i18n::t!(if raw {
                                        "container.detail.overview"
                                    } else {
                                        "container.field.inspect"
                                    })
                                    .to_string(),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(workspace) =
                                        this.state.container_workspace_mut(tab_id)
                                    {
                                        workspace.overview_raw = !raw;
                                        workspace.detail_tab = ContainerDetailTab::Overview;
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    let name = container.name.clone();
                    let running = container.state.is_running();
                    if container.state == transom_core::ContainerState::Paused {
                        actions = actions.child(
                            Button::new("container-unpause")
                                .xsmall()
                                .icon(IconName::Play)
                                .label(rust_i18n::t!("container.action.unpause").to_string())
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, _, cx| {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Unpause,
                                            cx,
                                        );
                                    }
                                })),
                        );
                    } else if running {
                        actions = actions
                            .child(
                                Button::new("container-pause")
                                    .xsmall()
                                    .icon(IconName::Pause)
                                    .label(rust_i18n::t!("container.action.pause").to_string())
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |this, _, _, cx| {
                                            this.container_lifecycle(
                                                tab_id,
                                                name.clone(),
                                                transom_core::ContainerLifecycleAction::Pause,
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(
                                Button::new("container-stop")
                                    .xsmall()
                                    .icon(IconName::Square)
                                    .label(rust_i18n::t!("container.action.stop").to_string())
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |this, _, _, cx| {
                                            this.container_lifecycle(
                                                tab_id,
                                                name.clone(),
                                                transom_core::ContainerLifecycleAction::Stop,
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(
                                Button::new("container-restart")
                                    .xsmall()
                                    .icon(IconName::RefreshCw)
                                    .label(rust_i18n::t!("container.action.restart").to_string())
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |this, _, _, cx| {
                                            this.container_lifecycle(
                                                tab_id,
                                                name.clone(),
                                                transom_core::ContainerLifecycleAction::Restart,
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(
                                Button::new("container-kill")
                                    .xsmall()
                                    .danger()
                                    .icon(IconName::CircleAlert)
                                    .label(rust_i18n::t!("container.action.kill").to_string())
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |this, _, _, cx| {
                                            this.container_lifecycle(
                                                tab_id,
                                                name.clone(),
                                                transom_core::ContainerLifecycleAction::Kill { signal: None },
                                                cx,
                                            );
                                        }
                                    })),
                            );
                    } else {
                        actions = actions.child(
                            Button::new("container-start")
                                .xsmall()
                                .icon(IconName::Play)
                                .label(rust_i18n::t!("container.action.start").to_string())
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, _, cx| {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Start,
                                            cx,
                                        );
                                    }
                                })),
                        );
                    }
                    actions = actions
                        .child(
                            Button::new("container-update")
                                .xsmall()
                                .icon(IconName::Settings)
                                .label(rust_i18n::t!("container.action.update").to_string())
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, window, cx| {
                                        this.open_container_update(tab_id, name.clone(), window, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new("container-remove")
                            .danger()
                            .xsmall()
                            .icon(IconName::Trash)
                            .label(rust_i18n::t!("common.delete").to_string())
                            .on_click(cx.listener({
                                let name = name.clone();
                                move |this, _, _, cx| {
                                    if let Some(workspace) =
                                        this.state.container_workspace_mut(tab_id)
                                    {
                                        workspace.confirm =
                                            Some(ContainerConfirm::Container(name.clone()));
                                    }
                                    cx.notify();
                                }
                            })),
                    );
                }
            }
            ContainerSection::Compose => {
                if let Some(project) = compose {
                    let names: Vec<String> = all_containers
                        .iter()
                        .filter(|item| {
                            item.compose_project.as_deref() == Some(project.name.as_str())
                        })
                        .map(|item| item.name.clone())
                        .collect();
                    let start_names = names.clone();
                    let stop_names = names;
                    actions = actions
                        .child(
                            Button::new("compose-start-all")
                                .xsmall()
                                .icon(IconName::Play)
                                .label(rust_i18n::t!("container.action.start_all").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    for name in &start_names {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Start,
                                            cx,
                                        );
                                    }
                                })),
                        )
                        .child(
                            Button::new("compose-stop-all")
                                .xsmall()
                                .icon(IconName::Square)
                                .label(rust_i18n::t!("container.action.stop_all").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    for name in &stop_names {
                                        this.container_lifecycle(
                                            tab_id,
                                            name.clone(),
                                            transom_core::ContainerLifecycleAction::Stop,
                                            cx,
                                        );
                                    }
                                })),
                        );
                    let compose_name = project.name.clone();
                    let up_name = compose_name.clone();
                    actions = actions
                        .child(
                            Button::new("compose-up")
                                .xsmall()
                                .icon(IconName::RotateCw)
                                .label(rust_i18n::t!("container.action.compose_up").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_compose(
                                        tab_id,
                                        up_name.clone(),
                                        transom_core::ComposeAction::Up,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("compose-down")
                                .xsmall()
                                .icon(IconName::Trash)
                                .label(rust_i18n::t!("container.action.compose_down").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_compose(
                                        tab_id,
                                        compose_name.clone(),
                                        transom_core::ComposeAction::Down,
                                        cx,
                                    );
                                })),
                        );
                }
            }
            ContainerSection::Images => {
                if let Some(image) = image {
                    let image_id = image.id.clone();
                    actions = actions
                        .child(
                            Button::new("image-tag")
                                .xsmall()
                                .icon(IconName::TagPlus)
                                .label(rust_i18n::t!("container.action.tag").to_string())
                                .on_click(cx.listener({
                                    let image_id = image_id.clone();
                                    move |this, _, window, cx| {
                                        this.open_image_tag(tab_id, image_id.clone(), window, cx);
                                    }
                                })),
                        )
                        .child(delete_button(
                            "image-delete",
                            cx,
                            tab_id,
                            ContainerConfirm::Image(image.id.clone()),
                        ));
                }
            }
            ContainerSection::Volumes => {
                if let Some(volume) = volume {
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
                    actions = actions.child(delete_button(
                        "network-delete",
                        cx,
                        tab_id,
                        ContainerConfirm::Network(network.id.clone()),
                    ));
                }
            }
        }

        // 有页签时页签占左、动作占右；没有页签的分类才显示标题，因为那里没有
        // 第二处能说明「当前选中的是谁」。
        let left: AnyElement = match tabs {
            Some(tabs) => tabs,
            None => h_flex()
                .gap_2()
                .items_center()
                .min_w_0()
                .child(Icon::new(section.icon()).small().text_color(p.primary))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .truncate()
                        .child(title),
                )
                .into_any_element(),
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
                    .children(health_chip)
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
    }
}
