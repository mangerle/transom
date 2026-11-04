//! 远端镜像市场探索与搜索弹窗界面。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与市场浏览（Hub）作为多源镜像中心呈现，支持在官方 Docker Hub 与用户
//!   自定义私有 Harbor / ACR 之间即时切换仓库源；
//! - 搜索结果清晰呈现官方认证（Official）徽标与社区星标数量（Stars），降低安全风险；
//! - 用户点击「下载」直接加入有界并发下载管理器，不阻塞前台界面并提供全链路分层进度跟踪。

use super::*;
use super::list::dropdown_trigger;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ContainerCommand, ContainerRegistry, ContainerRequestId, TabId};

impl AppShell {
    /// 打开远端镜像市场探索弹窗。
    pub(crate) fn open_container_hub_dialog(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_reg = app_settings(cx)
            .container_registries
            .iter()
            .find(|r| r.is_default)
            .map(|r| r.id.clone())
            .unwrap_or_else(|| ContainerRegistry::docker_hub().id);

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.hub.search_placeholder").to_string())
        });

        let subscription = cx.subscribe(&search_input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.execute_hub_search(tab_id, cx);
            }
        });

        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.hub_dialog = Some(ContainerHubDialog {
                search_input,
                selected_registry_id: default_reg,
                results: Vec::new(),
                searching: false,
                search_error: None,
                search_request: None,
                _subscription: subscription,
            });
        }
        cx.notify();
    }

    /// 发起远端镜像搜索请求。
    pub(crate) fn execute_hub_search(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let (term, _reg_id) = {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                return;
            };
            let Some(hub) = workspace.hub_dialog.as_ref() else {
                return;
            };
            let term = hub.search_input.read(cx).value().trim().to_string();
            let reg_id = hub.selected_registry_id.clone();
            (term, reg_id)
        };

        if term.is_empty() {
            return;
        }

        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if let Some(hub) = workspace.hub_dialog.as_mut() {
                hub.searching = true;
                hub.search_error = None;
                hub.search_request = Some(request_id);
            }
        }

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::SearchImages {
                tab_id,
                request_id,
                term,
                limit: Some(25),
            },
        );
        cx.notify();
    }

    /// 渲染远端镜像市场弹窗主体。
    pub(crate) fn render_container_hub_dialog_body(
        &self,
        tab_id: TabId,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let Some(hub) = workspace.hub_dialog.as_ref() else {
            return div().into_any_element();
        };

        let searching = hub.searching;
        let search_error = hub.search_error.clone();
        let results = hub.results.clone();
        let selected_reg_id = hub.selected_registry_id.clone();
        let registries = app_settings(cx).container_registries.clone();
        let search_input = hub.search_input.clone();

        let current_registry_name = registries
            .iter()
            .find(|r| r.id == selected_reg_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| rust_i18n::t!("container.hub.docker_hub_default").to_string());

        // 顶栏搜索栏与仓库切换
        let app_menu = app.clone();
        let reg_menu = dropdown_trigger(
            "container-hub-registry-select",
            current_registry_name.clone(),
            IconName::ChevronDown,
            None,
            move |menu, _, _| {
                let mut menu = menu;
                for reg in &registries {
                    let app_item = app_menu.clone();
                    let reg_id = reg.id.clone();
                    menu = menu.item(
                        PopupMenuItem::new(reg.name.clone())
                            .checked(reg.id == selected_reg_id)
                            .on_click(move |_, _, cx| {
                                app_item.update(cx, |this, cx| {
                                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                        if let Some(h) = ws.hub_dialog.as_mut() {
                                            h.selected_registry_id = reg_id.clone();
                                        }
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            },
        );

        let app_search = app.clone();
        let search_bar = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(reg_menu)
            .child(Input::new(&search_input).flex_1().min_w_0())
            .child(
                Button::new("container-hub-search-btn")
                    .primary()
                    .small()
                    .icon(if searching {
                        IconName::Loader
                    } else {
                        IconName::Search
                    })
                    .label(if searching {
                        rust_i18n::t!("container.hub.searching").to_string()
                    } else {
                        rust_i18n::t!("container.hub.search").to_string()
                    })
                    .on_click(move |_, _, cx| {
                        app_search.update(cx, |this, cx| {
                            this.execute_hub_search(tab_id, cx);
                        });
                    }),
            );

        // 搜索结果或加载/空状态展示
        let content: AnyElement = if searching {
            v_flex()
                .size_full()
                .h(px(280.0))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Loader)
                        .with_size(px(24.0))
                        .text_color(p.primary),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.searching_hint").to_string()),
                )
                .into_any_element()
        } else if let Some(err) = search_error {
            v_flex()
                .size_full()
                .h(px(200.0))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::CircleAlert)
                        .with_size(px(24.0))
                        .text_color(p.danger),
                )
                .child(div().text_sm().text_color(p.danger).child(err))
                .into_any_element()
        } else if results.is_empty() {
            v_flex()
                .size_full()
                .h(px(240.0))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Search)
                        .with_size(px(28.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.empty_hint").to_string()),
                )
                .into_any_element()
        } else {
            let mut list = v_flex().gap_2().w_full().p_1();
            for item in results {
                let name = item.name.clone();
                let desc = if item.description.is_empty() {
                    rust_i18n::t!("container.hub.no_description").to_string()
                } else {
                    item.description.clone()
                };
                let stars = item.star_count;
                let is_official = item.is_official;
                let pull_ref = item.name.clone();
                let reg_name = current_registry_name.clone();
                let app_pull = app.clone();

                let card = h_flex()
                    .w_full()
                    .p_3()
                    .gap_3()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(p.border)
                    .bg(p.background)
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(p.foreground)
                                            .child(name),
                                    )
                                    .when(is_official, |this| {
                                        this.child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .text_xs()
                                                .rounded(p.radius.sm)
                                                .bg(p.primary.opacity(0.15))
                                                .text_color(p.primary)
                                                .child(rust_i18n::t!("container.hub.official").to_string()),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.muted_foreground)
                                            .child(rust_i18n::t!("container.hub.stars", count = stars)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .line_clamp(2)
                                    .child(desc),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("hub-pull-{}", pull_ref).into()))
                            .secondary()
                            .small()
                            .icon(IconName::Download)
                            .label(rust_i18n::t!("container.hub.pull").to_string())
                            .on_click(move |_, window, cx| {
                                let pull_ref = pull_ref.clone();
                                let reg_name = reg_name.clone();
                                app_pull.update(cx, move |this, cx| {
                                    this.enqueue_image_pull(
                                        tab_id,
                                        pull_ref,
                                        reg_name,
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    );

                list = list.child(card);
            }

            div()
                .id("container-hub-scroll")
                .w_full()
                .max_h(px(400.0))
                .overflow_y_scroll()
                .child(list)
                .into_any_element()
        };

        let app_close = app.clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(search_bar)
            .child(content)
            .child(
                h_flex().justify_end().child(
                    Button::new("container-hub-close")
                        .ghost()
                        .small()
                        .label(rust_i18n::t!("container.hub.close").to_string())
                        .on_click(move |_, _, cx| {
                            app_close.update(cx, |this, cx| {
                                if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                    ws.hub_dialog = None;
                                }
                                cx.notify();
                            });
                        }),
                ),
            )
            .into_any_element()
    }

    /// 将选中的远端镜像加入下载队列并自动调度。
    pub(crate) fn enqueue_image_pull(
        &mut self,
        tab_id: TabId,
        reference: String,
        registry_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (task_id, should_start) = {
            let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
                return;
            };
            workspace
                .pull_manager
                .enqueue(tab_id, reference.clone(), registry_name)
        };

        // 自动打开下载抽屉以展现分层下载进度
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.pull_drawer_open = true;
        }

        if should_start {
            self.state.send_command(ContainerCommand::PullImageTask {
                tab_id,
                task_id,
                reference: reference.clone(),
                auth: None,
            });
            self.show_toast(
                rust_i18n::t!("container.hub.pull_started", reference = reference).to_string(),
                true,
                window,
                cx,
            );
        } else {
            self.show_toast(
                rust_i18n::t!("container.hub.pull_queued", reference = reference).to_string(),
                true,
                window,
                cx,
            );
        }
        cx.notify();
    }
}
