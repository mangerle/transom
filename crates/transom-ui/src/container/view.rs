//! 容器侧栏与管理工作区三栏视图。

use crate::app::AppShell;
use crate::container::types::{ContainerDiscovery, ContainerSection, ContainerWorkspaceStatus};
use crate::divider::{DividerEdge, DividerStyled, h_resizable};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 左侧槽位中的引擎连接列表。
    ///
    /// 设计原理（Why）：行首要能看到「这个连接现在通不通」，否则用户只能点开
    /// 标签才知道引擎没起来；连接摘要挪到名称下方两行式排版，尾部只留编辑与
    /// 删除两个动作，避免描述文字被挤到行尾截断。
    pub(crate) fn render_container_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let profiles = self.catalog.engines.clone();
        let states = self.container_profile_states();
        let discovery = self.containers.discovery.clone();
        let discovering = self.containers.discovering;

        let mut rows = v_flex().gap_0p5().p_2();
        if let Some(discovery) = discovery {
            rows = rows.child(self.render_discovery_panel(discovery, p, cx));
        }
        for profile in &profiles {
            let id = profile.id;
            let name = profile.display_name();
            let description = profile.engine.describe();
            let dot = states
                .get(&id)
                .map(|status| match status {
                    ContainerWorkspaceStatus::Connected => p.success,
                    ContainerWorkspaceStatus::Connecting => p.warning,
                    ContainerWorkspaceStatus::Failed(_)
                    | ContainerWorkspaceStatus::Disconnected => p.danger,
                })
                .unwrap_or(p.border);
            rows = rows.child(
                h_flex()
                    .id(ElementId::Name(format!("container-profile-{id}").into()))
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(p.radius.md)
                    .items_center()
                    .hover(|style| style.bg(p.muted.opacity(0.5)))
                    .child(div().size_2().flex_none().rounded_full().bg(dot))
                    .child(
                        Icon::new(IconName::Container)
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
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
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .truncate()
                                    .child(description),
                            ),
                    )
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
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("container-profile-new-empty")
                                    .primary()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(rust_i18n::t!("container.sidebar.new").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_container_form(None, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("container-discover-empty")
                                    .small()
                                    .icon(IconName::Radar)
                                    .label(rust_i18n::t!("container.sidebar.discover").to_string())
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.discover_engines(cx);
                                    })),
                            ),
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
                        h_flex()
                            .gap_0p5()
                            .child(
                                Button::new("container-discover")
                                    .ghost()
                                    .xsmall()
                                    .icon(if discovering {
                                        IconName::Loader
                                    } else {
                                        IconName::Radar
                                    })
                                    .tooltip(
                                        rust_i18n::t!("container.sidebar.discover").to_string(),
                                    )
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.discover_engines(cx);
                                    })),
                            )
                            .child(
                                Button::new("container-profile-new")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip(rust_i18n::t!("container.sidebar.new").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_container_form(None, window, cx);
                                    })),
                            ),
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

    /// 发现结果面板：列出可用引擎并给「添加并打开」，同时把不可用原因摊开。
    fn render_discovery_panel(
        &mut self,
        discovery: ContainerDiscovery,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ContainerDiscovery {
            candidates,
            failures,
        } = discovery;
        let mut body = v_flex()
            .gap_1()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.35));
        body = body.child(
            h_flex()
                .gap_1()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(IconName::Radar).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .child(rust_i18n::t!("container.discovery.title").to_string()),
                        ),
                )
                .child(
                    Button::new("container-discovery-dismiss")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.dismiss_container_discovery(cx);
                        })),
                ),
        );
        if candidates.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.discovery.none").to_string()),
            );
        }
        for candidate in candidates {
            let name = candidate.display_name();
            let endpoint = candidate.engine.describe();
            body = body.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_xs().truncate().child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .truncate()
                                    .child(endpoint),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(
                            format!("container-discovery-add-{}", candidate.id).into(),
                        ))
                        .primary()
                        .xsmall()
                        .icon(IconName::Plus)
                        .label(rust_i18n::t!("container.discovery.add").to_string())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.add_discovered_engine(candidate.clone(), window, cx);
                            },
                        )),
                    ),
            );
        }
        for failure in failures {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(failure),
            );
        }
        body.into_any_element()
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
        let list = self.render_container_list(tab_id, cx, p);
        let detail = self.render_container_detail(tab_id, window, cx, p);
        // 三栏宽度可拖拽：列表 320px 起步在长容器名/镜像名下会截断，详情区在
        // 小窗下又不够铺开；与主窗口其他分区一样交给用户拖。
        let (menu_split, list_split) = self
            .container_workspace(tab_id)
            .map(|workspace| (workspace.menu_split.clone(), workspace.list_split.clone()))
            .unwrap_or_else(|| {
                (
                    cx.new(|_| ResizableState::default()),
                    cx.new(|_| ResizableState::default()),
                )
            });

        let list_and_detail = h_resizable("container-split-list")
            .with_state(&list_split)
            .child(
                resizable_panel()
                    .size(px(340.0))
                    .size_range(px(240.0)..px(620.0))
                    .fixed()
                    .child(list),
            )
            .child(resizable_panel().child(detail));

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.background)
            .child(header)
            .child(
                h_resizable("container-split-menu")
                    .with_state(&menu_split)
                    .child(
                        resizable_panel()
                            .size(px(200.0))
                            .size_range(px(150.0)..px(320.0))
                            .fixed()
                            .child(
                                div()
                                    .id("container-menu-scroll")
                                    .size_full()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .border_color(p.divider(p.border))
                                    .divider_edge(DividerEdge::Right, p.divider_width)
                                    .child(menu),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(list_and_detail)
                            .into_any_element(),
                    ),
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
