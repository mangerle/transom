//! 容器侧栏与管理工作区三栏视图。

use crate::app::AppShell;
use crate::container::types::{ContainerSection, ContainerWorkspaceStatus};
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
        // 三栏宽度可拖拽：长容器名与镜像引用在固定宽度下必然截断，详情区在小窗下
        // 又铺不开，与主窗口其他分区一样交给用户拖。默认值只是首次打开的起点。
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
                    .size(px(380.0))
                    .size_range(px(260.0)..px(680.0))
                    .fixed()
                    // 面板本身不裁剪内容：工具条与行一旦超出宽度就整体压到右栏上
                    // （真实窗口实测），这里补一层裁剪容器把中栏锁在自己的宽度内。
                    .child(div().size_full().min_w_0().overflow_hidden().child(list)),
            )
            .child(resizable_panel().min_w_0().child(detail));

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
                                    .min_w_0()
                                    .min_h_0()
                                    .overflow_hidden()
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
        let dismiss = v_flex()
            .id("container-fullscreen-shell")
            .size_full()
            .bg(p.background)
            .key_context(crate::keymap::OVERLAY_CONTEXT)
            .on_key_down(
                cx.listener(move |this, event: &gpui_kit::KeyDownEvent, _, cx| {
                    this.on_container_overlay_key(event, Some(tab_id), cx);
                }),
            )
            .child(close)
            .child(div().flex_1().min_h_0().child(content));
        Self::overlay(dismiss, p)
    }
}
