//! 远端镜像市场探索与搜索弹窗界面。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与市场浏览（Hub）作为多源镜像中心呈现，支持在官方 Docker Hub 与用户
//!   自定义私有 Harbor / ACR 之间即时切换仓库源；
//! - 搜索结果清晰呈现官方认证（Official）徽标与社区星标数量（Stars），降低安全风险；
//! - 用户点击「下载」直接加入有界并发下载管理器，不阻塞前台界面并提供全链路分层进度跟踪。

use super::list::dropdown_trigger;
use super::*;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Icon, Sizable};
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
        let input_val = search_input.read(cx).value().trim().to_string();

        let current_registry_name = registries
            .iter()
            .find(|r| r.id == selected_reg_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| rust_i18n::t!("container.hub.docker_hub_default").to_string());

        // 1. 顶部标题栏与当前源指示
        let header = hub_view::render_hub_header(&current_registry_name, p);

        // 2. 顶栏搜索栏与仓库切换
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

        // 3. 热门探索芯片推荐行
        let app_chip = app.clone();
        let chips_bar = hub_view::render_popular_chips(
            move |tag, window, cx| {
                let tag = tag.to_string();
                let app_chip = app_chip.clone();
                app_chip.update(cx, move |this, cx| {
                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                        if let Some(hub) = ws.hub_dialog.as_mut() {
                            hub.search_input.update(cx, |input, cx| {
                                input.set_value(tag, window, cx);
                            });
                        }
                    }
                    this.execute_hub_search(tab_id, cx);
                });
            },
            p,
        );

        // 4. 内容主体根据检索状态切换
        let content: AnyElement = if searching {
            v_flex()
                .size_full()
                .h(px(320.0))
                .items_center()
                .justify_center()
                .gap_2p5()
                .child(
                    v_flex()
                        .w(px(44.0))
                        .h(px(44.0))
                        .rounded(p.radius.full)
                        .bg(p.primary.opacity(0.10))
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Loader)
                                .with_size(px(22.0))
                                .text_color(p.primary),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("container.hub.searching_hint").to_string()),
                )
                .into_any_element()
        } else if let Some(err) = search_error {
            let app_retry = app.clone();
            v_flex()
                .size_full()
                .h(px(260.0))
                .items_center()
                .justify_center()
                .gap_3()
                .p_4()
                .rounded(p.radius.lg)
                .bg(p.danger.opacity(0.04))
                .border_1()
                .border_color(p.danger.opacity(0.18))
                .child(
                    v_flex()
                        .w(px(40.0))
                        .h(px(40.0))
                        .rounded(p.radius.full)
                        .bg(p.danger.opacity(0.12))
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::CircleAlert)
                                .with_size(px(20.0))
                                .text_color(p.danger),
                        ),
                )
                .child(div().text_sm().text_color(p.danger).child(err))
                .child(
                    Button::new("container-hub-retry-btn")
                        .secondary()
                        .small()
                        .icon(IconName::Search)
                        .label(rust_i18n::t!("container.hub.search").to_string())
                        .on_click(move |_, _, cx| {
                            app_retry.update(cx, |this, cx| {
                                this.execute_hub_search(tab_id, cx);
                            });
                        }),
                )
                .into_any_element()
        } else if results.is_empty() {
            if input_val.is_empty() {
                hub_view::render_hub_welcome_guide(p)
            } else {
                v_flex()
                    .size_full()
                    .h(px(280.0))
                    .items_center()
                    .justify_center()
                    .gap_2p5()
                    .child(
                        v_flex()
                            .w(px(44.0))
                            .h(px(44.0))
                            .rounded(p.radius.full)
                            .bg(p.muted.opacity(0.08))
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Search)
                                    .with_size(px(22.0))
                                    .text_color(p.muted_foreground),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.hub.no_description").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("container.hub.empty_hint").to_string()),
                    )
                    .into_any_element()
            }
        } else {
            let mut list = v_flex().gap_2().w_full().p_1();
            let results_header =
                hub_view::render_hub_results_header(results.len(), &current_registry_name, p);
            list = list.child(results_header);

            for item in results {
                let pull_ref = item.name.clone();
                let reg_name = current_registry_name.clone();
                let app_pull = app.clone();

                let card = hub_view::render_hub_result_card(
                    &item,
                    move |window, cx| {
                        let pull_ref = pull_ref.clone();
                        let reg_name = reg_name.clone();
                        app_pull.update(cx, move |this, cx| {
                            this.enqueue_image_pull(tab_id, pull_ref, reg_name, window, cx);
                        });
                    },
                    p,
                );

                list = list.child(card);
            }

            div()
                .id("container-hub-scroll")
                .w_full()
                .max_h(px(440.0))
                .overflow_y_scroll()
                .child(list)
                .into_any_element()
        };

        // 5. 底部 Footer
        let app_close = app.clone();
        let footer = hub_view::render_hub_footer(
            move |_, cx| {
                app_close.update(cx, |this, cx| {
                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                        ws.hub_dialog = None;
                    }
                    cx.notify();
                });
            },
            p,
        );

        v_flex()
            .w_full()
            .gap_3()
            .child(header)
            .child(search_bar)
            .child(chips_bar)
            .child(content)
            .child(footer)
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
