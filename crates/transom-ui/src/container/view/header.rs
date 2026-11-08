//! 容器工作区头部：引擎标识、操作按钮与分类页签。
//!
//! 设计原理（Why）：头部只依赖工作区实体、只读状态快照与 `Palette`；切换分类、
//! 刷新列表与拉取抽屉开关都直接 `update` 实体，全屏与弹窗这类需要外壳参与的
//! 动作经 [`ContainerWorkspaceEvent`] 上报。渲染函数不再挂在 `AppShell` 上，
//! 也不需要按 `tab_id` 反查状态。

use crate::container::divider_width_px;
use crate::container::types::ContainerSection;
use crate::container::{ContainerWorkspace, ContainerWorkspaceEvent, ContainerWorkspaceState};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Tab, Tabs, h_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// 渲染容器工作区头部。
///
/// 设计原理（Why）：外壳只读一次实体状态后把句柄与快照传进来，头部不再反查
/// `AppShell`。`fullscreen` 由外壳的 `ContainerUiState` 持有（全屏覆盖层也在外壳
/// 渲染），因此以布尔快照注入，仅用于切换按钮的图标。
pub(super) fn render_container_header(
    view: &Entity<ContainerWorkspace>,
    workspace: &ContainerWorkspaceState,
    fullscreen: bool,
    p: &Palette,
) -> AnyElement {
    let section_tabs = render_section_tabs(view, workspace, p);
    let identity = render_container_engine_identity(workspace, p);
    let actions = render_container_tab_actions(view, workspace, fullscreen, p);

    h_flex()
        .relative()
        .h(rems(crate::theme::tokens::size::TAB_BAR_HEIGHT))
        .pr_2()
        .gap_2()
        .items_center()
        .justify_between()
        .flex_none()
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(divider_width_px(p.divider_width)))
                .bg(p.divider(p.border)),
        )
        .child(section_tabs)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_end()
                .min_w_0()
                .flex_shrink_1()
                .child(identity)
                .child(actions),
        )
        .into_any_element()
}

/// 引擎身份摘要：图标、名称与 API/平台信息。
fn render_container_engine_identity(
    workspace: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let profile = &workspace.profile;
    let title = profile.display_name();
    let icon = crate::session_appearance::session_icon(profile.icon)
        .unwrap_or(crate::app_assets::AppIcon::Named(IconName::Boxes));
    let profile_color = crate::session_appearance::session_color(profile.color);
    let info = workspace.info.as_ref().map(|info| {
        format!(
            "{} · API {} · {}",
            info.title(),
            info.api_version,
            info.platform
        )
    });

    h_flex()
        .gap_2()
        .items_center()
        .min_w_0()
        .flex_shrink_1()
        .child(
            Icon::new(icon)
                .small()
                .text_color(profile_color.unwrap_or(p.primary)),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .truncate()
                .child(title),
        )
        .when_some(info, |this, info| {
            this.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .truncate()
                    .child(info),
            )
        })
        .into_any_element()
}

/// 头部右侧操作按钮：拉取抽屉、Pod、Quadlet、磁盘、刷新与全屏。
fn render_container_tab_actions(
    view: &Entity<ContainerWorkspace>,
    workspace: &ContainerWorkspaceState,
    fullscreen: bool,
    p: &Palette,
) -> AnyElement {
    let busy = workspace.busy;
    let pulling_count = workspace.pull_manager.pulling_count();
    let drawer_open = workspace.pull_drawer_open;
    let has_tasks = !workspace.pull_manager.tasks.is_empty();
    let is_podman = workspace
        .info
        .as_ref()
        .is_some_and(|info| info.engine.kind == transom_core::ContainerEngineKind::Podman);
    let is_local_engine = workspace.info.as_ref().is_some_and(|info| {
        matches!(
            info.engine.endpoint,
            transom_core::ContainerEndpoint::Auto
                | transom_core::ContainerEndpoint::Unix { .. }
                | transom_core::ContainerEndpoint::NamedPipe { .. }
                | transom_core::ContainerEndpoint::Cli { .. }
        )
    });

    h_flex()
        .gap_1()
        .items_center()
        .flex_shrink_0()
        .when(busy, |this| {
            this.child(
                Icon::new(IconName::Loader)
                    .small()
                    .text_color(p.muted_foreground),
            )
        })
        .when(has_tasks || pulling_count > 0, |this| {
            let btn = if pulling_count > 0 {
                Button::new("container-pull-drawer-btn")
                    .primary()
                    .xsmall()
                    .icon(IconName::Download)
                    .label(rust_i18n::t!(
                        "container.pull_manager.indicator",
                        count = pulling_count
                    ))
            } else {
                Button::new("container-pull-drawer-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Download)
                    .tooltip(if drawer_open {
                        rust_i18n::t!("container.pull_manager.drawer_collapse")
                    } else {
                        rust_i18n::t!("container.pull_manager.drawer_expand")
                    })
            };
            let view = view.clone();
            this.child(btn.on_click(move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.state.pull_drawer_open = !workspace.state.pull_drawer_open;
                    cx.notify();
                });
            }))
        })
        .when(is_podman, |this| {
            let view = view.clone();
            this.child(
                Button::new("container-pods-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Layers)
                    .tooltip(rust_i18n::t!("container.pods.title").to_string())
                    .on_click(move |_, _, cx| {
                        view.update(cx, |_workspace, cx| {
                            cx.emit(ContainerWorkspaceEvent::OpenPods);
                        });
                    }),
            )
        })
        .when(is_podman && is_local_engine, |this| {
            let view = view.clone();
            this.child(
                Button::new("container-quadlet-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Settings)
                    .tooltip(rust_i18n::t!("container.quadlet.title").to_string())
                    .on_click(move |_, _, cx| {
                        view.update(cx, |_workspace, cx| {
                            cx.emit(ContainerWorkspaceEvent::OpenQuadlet);
                        });
                    }),
            )
        })
        .child({
            let view = view.clone();
            Button::new("container-system-df-btn")
                .ghost()
                .xsmall()
                .icon(IconName::HardDrive)
                .tooltip(rust_i18n::t!("container.system_df.title").to_string())
                .on_click(move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::OpenSystemDf);
                    });
                })
        })
        .child({
            let view = view.clone();
            Button::new("container-refresh")
                .ghost()
                .xsmall()
                .icon(IconName::RefreshCw)
                .on_click(move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.refresh_all();
                        cx.notify();
                    });
                })
        })
        .child({
            let view = view.clone();
            Button::new("container-fullscreen")
                .ghost()
                .xsmall()
                .icon(if fullscreen {
                    IconName::Minimize
                } else {
                    IconName::Expand
                })
                .on_click(move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::ToggleFullscreen);
                    });
                })
        })
        .into_any_element()
}

/// 分类页签：计数来自工作区快照，点击后由实体切换分类并刷新列表。
fn render_section_tabs(
    view: &Entity<ContainerWorkspace>,
    workspace: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let selected = workspace.section;
    let container_count = if workspace.include_compose {
        workspace.containers.len()
    } else {
        workspace
            .containers
            .iter()
            .filter(|c| c.compose_project.is_none())
            .count()
    };
    let counts = [
        workspace.compose.len(),
        container_count,
        workspace.images.len(),
        workspace.volumes.len(),
        workspace.networks.len(),
    ];

    let tab_count = ContainerSection::ALL.len();
    let divider_px = divider_width_px(p.divider_width);
    let active_line_px = divider_px.max(2.0);
    Tabs::new("container-section-tabs")
        .h_full()
        .flex()
        .items_center()
        .flex_shrink_0()
        .children(
            ContainerSection::ALL
                .into_iter()
                .zip(counts)
                .enumerate()
                .map(|(index, (section, count))| {
                    let is_selected = section == selected;
                    let icon_color = if is_selected {
                        p.primary
                    } else {
                        p.muted_foreground
                    };
                    let text_color = if is_selected {
                        p.foreground
                    } else {
                        p.muted_foreground
                    };
                    let mut tab = Tab::new(ElementId::Name(
                        format!("container-section-{}", section.label_key()).into(),
                    ))
                    .relative()
                    .h_full()
                    .px_3()
                    .gap_1p5()
                    .items_center()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(text_color)
                    .set_position(index + 1, tab_count);

                    if !is_selected {
                        tab = tab.hover(|style| style.text_color(p.foreground));
                    }

                    tab.child(Icon::new(section.icon()).small().text_color(icon_color))
                        .child(rust_i18n::t!(section.label_key()).to_string())
                        .when(count > 0, |this| {
                            this.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(p.muted_foreground)
                                    .child(count.to_string()),
                            )
                        })
                        .when(is_selected, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(px(active_line_px))
                                    .bg(p.primary),
                            )
                        })
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |workspace, cx| {
                                    workspace.select_container_section(
                                        workspace.tab_id,
                                        section,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        })
                }),
        )
        .into_any_element()
}
