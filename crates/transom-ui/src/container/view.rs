//! 容器管理工作区：顶部工作栏与列表/详情双栏视图。

use crate::app::AppShell;
use crate::container::divider_width_px;
use crate::container::types::{ContainerSection, ContainerWorkspaceStatus};
use crate::divider::{DividerEdge, DividerStyled, h_resizable};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Tab, Tabs, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 管理工作区标签的完整视图。
    pub(crate) fn render_container_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        if !matches!(workspace.status, ContainerWorkspaceStatus::Connected) {
            return self.render_container_connection_state(tab_id, window, cx, p);
        }
        self.flush_container_notice(tab_id, window, cx);
        let header = self.render_container_header(tab_id, window, cx, p);
        let list = self.render_container_list(tab_id, cx, p);
        let detail = self.render_container_detail(tab_id, window, cx, p);
        // 列表与详情宽度可拖拽：长容器名与镜像引用在固定宽度下必然截断，详情区在
        // 小窗下又铺不开，与主窗口其他分区一样交给用户拖。默认值只是首次打开的起点。
        let list_split = self
            .state
            .container_workspace(tab_id)
            .map(|workspace| workspace.list_split.clone())
            .unwrap_or_else(|| cx.new(|_| ResizableState::default()));

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
            .child(div().flex_1().min_w_0().min_h_0().child(list_and_detail))
            .into_any_element()
    }

    /// 把容器工作区暂存的操作提示转成右上角通知。
    ///
    /// 提示在动作发生时写入状态，渲染帧取走后再推送通知；这样不需要让每个
    /// 业务动作都持有窗口句柄，也不会在连续重绘中重复弹出同一条消息。
    fn flush_container_notice(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let notice = self
            .state
            .container_workspace_mut(tab_id)
            .and_then(|workspace| workspace.notice.take());
        if let Some(message) = notice {
            self.show_info_toast(message, window, cx);
        }
    }

    fn render_container_connection_state(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
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
                        this.state
                            .send_command(transom_core::ContainerCommand::Connect {
                                tab_id,
                                profile: profile.clone(),
                            });
                        if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
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

    fn render_container_engine_identity(&self, tab_id: TabId, p: &Palette) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
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
            .into_any_element()
    }

    fn render_container_tab_actions(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let busy = workspace.busy;
        let fullscreen = self.state.containers.fullscreen == Some(tab_id);

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

    fn render_container_section_tabs(
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
        let counts = [
            workspace.compose.len(),
            workspace.containers.len(),
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
                            p.tab_active_foreground
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
                        .border_color(p.divider(p.border))
                        .when(index > 0, |this| {
                            this.divider_edge(DividerEdge::Left, p.divider_width)
                        })
                        .when(index + 1 == tab_count, |this| {
                            this.divider_edge(DividerEdge::Right, p.divider_width)
                        })
                        .set_position(index + 1, tab_count);
                        if is_selected {
                            tab = tab.bg(p.tab_active).text_color(p.tab_active_foreground);
                        } else {
                            tab = tab.text_color(p.muted_foreground).hover(|style| {
                                style
                                    .bg(p.sidebar_accent.opacity(0.6))
                                    .text_color(p.foreground)
                            });
                        }
                        tab.child(Icon::new(section.icon()).small().text_color(icon_color))
                            .child(rust_i18n::t!(section.label_key()).to_string())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if is_selected {
                                        p.tab_active_foreground.opacity(0.72)
                                    } else {
                                        p.muted_foreground
                                    })
                                    .child(count.to_string()),
                            )
                            .child({
                                let (line_height, line_color) = if is_selected {
                                    (active_line_px, p.primary)
                                } else {
                                    (divider_px, p.divider(p.border))
                                };
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(px(line_height))
                                    .bg(line_color)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_container_section(tab_id, section, window, cx);
                            }))
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
        if self.state.container_workspace(tab_id).is_none() {
            return div().into_any_element();
        }
        let content = self.render_container_tab(tab_id, window, cx, p);
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
            .child(content);
        Self::overlay(dismiss, p)
    }
}
