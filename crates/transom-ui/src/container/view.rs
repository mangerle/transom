//! 容器侧栏与管理工作区三栏视图。

use crate::app::AppShell;
use crate::container::types::{ContainerSection, ContainerWorkspaceStatus};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 左侧槽位中的引擎连接列表。
    pub(crate) fn render_container_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let profiles = self.catalog.engines.clone();
        let mut rows = v_flex().gap_0p5().p_2();
        for profile in &profiles {
            let id = profile.id;
            let name = profile.display_name();
            let description = profile.engine.describe();
            rows = rows.child(
                h_flex()
                    .id(ElementId::Name(format!("container-profile-{id}").into()))
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(p.radius.md)
                    .items_center()
                    .hover(|style| style.bg(p.muted.opacity(0.5)))
                    .child(Icon::new(IconName::Container).small().text_color(p.primary))
                    .child(
                        Button::new(ElementId::Name(
                            format!("container-profile-open-{id}").into(),
                        ))
                        .ghost()
                        .label(name)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if let Some(profile) = this
                                    .catalog
                                    .engines
                                    .iter()
                                    .find(|profile| profile.id == id)
                                    .cloned()
                                {
                                    this.open_engine_workspace(profile, window, cx);
                                }
                            },
                        )),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(ElementId::Name(
                            format!("container-profile-edit-{id}").into(),
                        ))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Pen)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                let profile = this
                                    .catalog
                                    .engines
                                    .iter()
                                    .find(|profile| profile.id == id)
                                    .cloned();
                                this.open_container_form(profile, window, cx);
                            },
                        )),
                    )
                    .child(
                        Button::new(ElementId::Name(
                            format!("container-profile-delete-{id}").into(),
                        ))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.delete_container_profile(id, window, cx);
                            },
                        )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(description),
                    ),
            );
        }
        if profiles.is_empty() {
            rows = rows.child(
                v_flex()
                    .gap_2()
                    .p_2()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.sidebar.empty").to_string())
                    .child(
                        Button::new("container-profile-new-empty")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("container.sidebar.new").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_container_form(None, window, cx);
                            })),
                    ),
            );
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(rems(crate::theme::tokens::size::TAB_BAR_HEIGHT))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .border_color(p.divider(p.border))
                    .divider_edge(DividerEdge::Bottom, p.divider_width)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(IconName::Container).small().text_color(p.primary))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child(rust_i18n::t!("container.title").to_string()),
                            ),
                    )
                    .child(
                        Button::new("container-profile-new")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_container_form(None, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("container-profiles-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .into_any_element()
    }

    /// 管理工作区标签的完整三栏视图。
    pub(crate) fn render_container_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        if !matches!(workspace.status, ContainerWorkspaceStatus::Connected) {
            return self.render_container_connection_state(tab_id, window, cx, p);
        }
        let header = self.render_container_header(tab_id, window, cx, p);
        let menu = self.render_container_menu(tab_id, window, cx, p);
        let list = self.render_container_list(tab_id, window, cx, p);
        let detail = self.render_container_detail(tab_id, window, cx, p);

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.background)
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("container-menu-scroll")
                            .w(px(190.0))
                            .h_full()
                            .flex_none()
                            .overflow_y_scroll()
                            .border_color(p.divider(p.border))
                            .divider_edge(DividerEdge::Right, p.divider_width)
                            .child(menu),
                    )
                    .child(
                        div()
                            .w(px(320.0))
                            .h_full()
                            .flex_none()
                            .min_h_0()
                            .overflow_hidden()
                            .border_color(p.divider(p.border))
                            .divider_edge(DividerEdge::Right, p.divider_width)
                            .child(list),
                    )
                    .child(div().flex_1().h_full().min_w_0().child(detail)),
            )
            .into_any_element()
    }

    fn render_container_connection_state(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let status = workspace.status.clone();
        let profile = workspace.profile.clone();
        let (icon, title, detail, retry) = match status {
            ContainerWorkspaceStatus::Connecting => (
                IconName::Loader,
                rust_i18n::t!("container.state.connecting").to_string(),
                profile.engine.describe(),
                false,
            ),
            ContainerWorkspaceStatus::Failed(error) => (
                IconName::CircleAlert,
                rust_i18n::t!("container.state.failed").to_string(),
                error,
                true,
            ),
            ContainerWorkspaceStatus::Disconnected => (
                IconName::CircleAlert,
                rust_i18n::t!("container.state.disconnected").to_string(),
                profile.engine.describe(),
                true,
            ),
            ContainerWorkspaceStatus::Connected => {
                (IconName::Container, String::new(), String::new(), false)
            }
        };
        let mut body = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                Icon::new(icon)
                    .with_size(px(28.0))
                    .text_color(p.muted_foreground),
            )
            .child(div().text_sm().child(title))
            .child(div().text_xs().text_color(p.muted_foreground).child(detail));
        if retry {
            body = body.child(
                Button::new("container-retry")
                    .primary()
                    .small()
                    .icon(IconName::RefreshCw)
                    .label(rust_i18n::t!("common.retry").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.send_command(transom_core::ContainerCommand::Connect {
                            tab_id,
                            profile: profile.clone(),
                        });
                        if let Some(workspace) = this.container_workspace_mut(tab_id) {
                            workspace.status = ContainerWorkspaceStatus::Connecting;
                        }
                        cx.notify();
                    })),
            );
        }
        body.into_any_element()
    }

    fn render_container_header(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let title = workspace.profile.display_name();
        let info = workspace.info.as_ref().map(|info| {
            format!(
                "{} · API {} · {}",
                info.title(),
                info.api_version,
                info.platform
            )
        });
        let busy = workspace.busy;
        let notice = workspace.notice.clone();
        let fullscreen = self.containers.fullscreen == Some(tab_id);

        h_flex()
            .h(rems(crate::theme::tokens::size::TAB_BAR_HEIGHT))
            .px_3()
            .gap_3()
            .items_center()
            .justify_between()
            .flex_none()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .flex_shrink_1()
                    .child(Icon::new(IconName::Container).small().text_color(p.primary))
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
                    .when_some(notice, |this, notice| {
                        this.child(div().text_xs().text_color(p.muted_foreground).child(notice))
                    }),
            )
            .child(
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
                    .child(
                        Button::new("container-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.refresh_container_section(tab_id);
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
                                this.containers.fullscreen =
                                    if fullscreen { None } else { Some(tab_id) };
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_container_menu(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let selected = workspace.section;
        let counts = [
            workspace.compose.len(),
            workspace.containers.len(),
            workspace.images.len(),
            workspace.volumes.len(),
            workspace.networks.len(),
        ];

        v_flex()
            .gap_0p5()
            .p_2()
            .children(
                ContainerSection::ALL
                    .into_iter()
                    .zip(counts)
                    .map(|(section, count)| {
                        let is_selected = section == selected;
                        let mut button = Button::new(ElementId::Name(
                            format!("container-section-{}", section.label_key()).into(),
                        ))
                        .ghost()
                        .label(rust_i18n::t!(section.label_key()).to_string())
                        .icon(section.icon());
                        if is_selected {
                            button = button.selected(true);
                        }
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(button.on_click(cx.listener(move |this, _, window, cx| {
                                this.select_container_section(tab_id, section, window, cx);
                            })))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(count.to_string()),
                            )
                    }),
            )
            .into_any_element()
    }

    fn render_container_list(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let section = workspace.section;
        let search = workspace.search.clone();
        let create_input = workspace.create_input.clone();
        let running_only = workspace.running_only;
        let query = workspace.query(cx).to_lowercase();
        let busy = workspace.busy;
        let selected = workspace.selected.clone();
        let containers: Vec<_> = workspace
            .containers
            .iter()
            .filter(|item| !running_only || item.state.is_running())
            .filter(|item| {
                query.is_empty()
                    || item.name.to_lowercase().contains(&query)
                    || item.image.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let images = workspace.images.clone();
        let volumes = workspace.volumes.clone();
        let networks = workspace.networks.clone();
        let compose = workspace.compose.clone();

        let mut toolbar = h_flex()
            .gap_1()
            .items_center()
            .child(Input::new(&search).flex_1());
        if section == ContainerSection::Containers {
            toolbar = toolbar.child(
                Button::new("container-filter-running")
                    .xsmall()
                    .selected(running_only)
                    .label(rust_i18n::t!("container.filter.running").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(workspace) = this.container_workspace_mut(tab_id) {
                            workspace.running_only = !workspace.running_only;
                        }
                        cx.notify();
                    })),
            );
        }

        let mut rows = v_flex().gap_0p5();
        match section {
            ContainerSection::Containers => {
                for item in &containers {
                    let id = item.id.clone();
                    let is_selected = selected.as_deref() == Some(id.as_str());
                    rows = rows.child(container_row(
                        cx,
                        ContainerRow {
                            id: ElementId::Name(format!("container-row-{id}").into()),
                            selected: is_selected,
                            icon: IconName::Container,
                            title: item.name.clone(),
                            subtitle: format!("{} · {}", item.image, item.status),
                            badge: item.state.key(),
                            palette: p,
                        },
                        move |this, _cx| this.select_container_item(tab_id, id.clone()),
                    ));
                }
            }
            ContainerSection::Images => {
                for item in &images {
                    let id = item.id.clone();
                    let is_selected = selected.as_deref() == Some(id.as_str());
                    rows = rows.child(container_row(
                        cx,
                        ContainerRow {
                            id: ElementId::Name(format!("image-row-{id}").into()),
                            selected: is_selected,
                            icon: IconName::Layers,
                            title: item.display_name(),
                            subtitle: format!(
                                "{} · {}",
                                crate::format::format_size(item.size),
                                short(&item.id)
                            ),
                            badge: "image",
                            palette: p,
                        },
                        move |this, _cx| this.select_image_item(tab_id, id.clone()),
                    ));
                }
            }
            ContainerSection::Volumes => {
                for item in &volumes {
                    let name = item.name.clone();
                    let is_selected = selected.as_deref() == Some(name.as_str());
                    rows = rows.child(container_row(
                        cx,
                        ContainerRow {
                            id: ElementId::Name(format!("volume-row-{name}").into()),
                            selected: is_selected,
                            icon: IconName::Database,
                            title: name.clone(),
                            subtitle: format!("{} · {}", item.driver, item.mountpoint),
                            badge: "volume",
                            palette: p,
                        },
                        move |this, _cx| this.select_volume_item(tab_id, name.clone()),
                    ));
                }
            }
            ContainerSection::Networks => {
                for item in &networks {
                    let id = item.id.clone();
                    let is_selected = selected.as_deref() == Some(id.as_str());
                    rows = rows.child(container_row(
                        cx,
                        ContainerRow {
                            id: ElementId::Name(format!("network-row-{id}").into()),
                            selected: is_selected,
                            icon: IconName::Network,
                            title: item.name.clone(),
                            subtitle: format!(
                                "{} · {}{}",
                                item.driver,
                                item.scope,
                                item.subnet
                                    .as_ref()
                                    .map(|subnet| format!(" · {subnet}"))
                                    .unwrap_or_default()
                            ),
                            badge: "network",
                            palette: p,
                        },
                        move |this, _cx| this.select_network_item(tab_id, id.clone()),
                    ));
                }
            }
            ContainerSection::Compose => {
                for project in &compose {
                    let name = project.name.clone();
                    let is_selected = selected.as_deref() == Some(name.as_str());
                    rows = rows.child(container_row(
                        cx,
                        ContainerRow {
                            id: ElementId::Name(format!("compose-row-{name}").into()),
                            selected: is_selected,
                            icon: IconName::Boxes,
                            title: name.clone(),
                            subtitle: format!(
                                "{} · {}",
                                project.state_label(),
                                project.working_dir.clone().unwrap_or_default()
                            ),
                            badge: "compose",
                            palette: p,
                        },
                        move |this, _cx| this.select_compose_item(tab_id, name.clone()),
                    ));
                }
            }
        }

        let mut body = v_flex().h_full().child(
            v_flex()
                .gap_1p5()
                .p_2()
                .flex_none()
                .child(toolbar)
                .when(
                    matches!(
                        section,
                        ContainerSection::Images
                            | ContainerSection::Volumes
                            | ContainerSection::Networks
                    ),
                    |this| {
                        this.child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(Input::new(&create_input).flex_1())
                                .child(
                                    Button::new("container-create-submit")
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::Plus)
                                        .label(if section == ContainerSection::Images {
                                            rust_i18n::t!("container.pull").to_string()
                                        } else {
                                            rust_i18n::t!("container.create").to_string()
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.submit_container_create(tab_id, window, cx);
                                        })),
                                ),
                        )
                    },
                )
                .child(
                    div()
                        .id("container-list-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(rows),
                ),
        );
        if busy {
            body = body.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.loading").to_string()),
            );
        }
        body.into_any_element()
    }

    /// 全屏模态：复用同一份工作区状态，标签内显示占位避免重复渲染终端。
    pub(crate) fn render_container_fullscreen(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        if self.container_workspace(tab_id).is_none() {
            return div().into_any_element();
        }
        let content = self.render_container_tab(tab_id, window, cx, p);
        let close = h_flex().justify_end().p_2().child(
            Button::new("container-fullscreen-close")
                .ghost()
                .small()
                .icon(IconName::Minimize)
                .label(rust_i18n::t!("container.fullscreen_exit").to_string())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.containers.fullscreen = None;
                    cx.notify();
                })),
        );
        Self::overlay(
            v_flex()
                .size_full()
                .bg(p.background)
                .child(close)
                .child(div().flex_1().min_h_0().child(content)),
            p,
        )
    }
}

fn short(id: &str) -> String {
    transom_core::short_id(id)
}

/// 列表行的渲染参数：字段较多，收敛为结构体避免长参数列表。
struct ContainerRow<'a> {
    id: ElementId,
    selected: bool,
    icon: IconName,
    title: String,
    subtitle: String,
    badge: &'static str,
    palette: &'a Palette,
}

fn container_row(
    cx: &mut Context<AppShell>,
    row: ContainerRow<'_>,
    on_select: impl Fn(&mut AppShell, &mut Context<AppShell>) + 'static,
) -> AnyElement {
    let p = row.palette;
    let mut element = h_flex()
        .id(row.id)
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(p.radius.md)
        .items_center()
        .cursor_pointer()
        .hover(|style| style.bg(p.muted.opacity(0.5)))
        .child(Icon::new(row.icon).small().text_color(p.muted_foreground))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().truncate().child(row.title))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .truncate()
                        .child(row.subtitle),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(row.badge.to_string()),
        );
    if row.selected {
        element = element
            .bg(p.sidebar_accent)
            .text_color(p.sidebar_accent_foreground);
    }
    element
        .on_click(cx.listener(move |this, _, _, cx| {
            on_select(this, cx);
            cx.notify();
        }))
        .into_any_element()
}
