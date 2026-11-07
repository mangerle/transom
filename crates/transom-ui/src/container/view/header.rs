//! 容器工作区头部：引擎标识、操作按钮与分类页签。

use crate::app::AppShell;
use crate::container::divider_width_px;
use crate::container::types::ContainerSection;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Tab, Tabs, h_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    pub(super) fn render_container_header(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let section_tabs = self.render_container_section_tabs(tab_id, window, cx, p);
        let identity = self.render_container_engine_identity(tab_id, p);
        let actions = self.render_container_tab_actions(tab_id, cx, p);

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

    pub(super) fn render_container_engine_identity(
        &self,
        tab_id: TabId,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
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

    pub(super) fn render_container_tab_actions(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let busy = workspace.busy;
        let pulling_count = workspace.pull_manager.pulling_count();
        let drawer_open = workspace.pull_drawer_open;
        let has_tasks = !workspace.pull_manager.tasks.is_empty();
        let fullscreen = self.state.containers.fullscreen == Some(tab_id);
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
                this.child(btn.on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                        ws.pull_drawer_open = !ws.pull_drawer_open;
                    }
                    cx.notify();
                })))
            })
            .when(is_podman, |this| {
                this.child(
                    Button::new("container-pods-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Layers)
                        .tooltip(rust_i18n::t!("container.pods.title").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_container_pods_dialog(tab_id, cx);
                        })),
                )
            })
            .when(is_podman && is_local_engine, |this| {
                this.child(
                    Button::new("container-quadlet-btn")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Settings)
                        .tooltip(rust_i18n::t!("container.quadlet.title").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_container_quadlets_dialog(tab_id, cx);
                        })),
                )
            })
            .child(
                Button::new("container-system-df-btn")
                    .ghost()
                    .xsmall()
                    .icon(IconName::HardDrive)
                    .tooltip(rust_i18n::t!("container.system_df.title").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_container_system_df_dialog(tab_id, cx);
                    })),
            )
            .child(
                Button::new("container-refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.refresh_container_workspace_all(tab_id);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("container-fullscreen")
                    .ghost()
                    .xsmall()
                    .icon(if fullscreen {
                        IconName::Minimize
                    } else {
                        IconName::Expand
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.containers.fullscreen =
                            if fullscreen { None } else { Some(tab_id) };
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_container_section_tabs(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_container_section(tab_id, section, window, cx);
                            }))
                    }),
            )
            .into_any_element()
    }
}
