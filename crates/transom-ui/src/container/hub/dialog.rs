//! 镜像仓库探索弹窗主体渲染与状态分流装配。
//!
//! 设计原理（Why）：
//! - 将顶层弹窗装配与状态分流（详情视图 vs 搜索结果列表）从业务动作中拆离；
//! - 采用纯声明式布局，各子区块均由独立纯组件提供；
//! - 严格遵守单文件行数红线与模块职责分离准则。
//!
//! 动作归属（Why）：搜索、详情、版本选择与入队都归工作区实体；本函数只读取
//! 状态快照，所有交互回调直接 `update` 实体，不再经外壳转发。

use super::card::render_hub_result_card;
use super::detail::render_hub_image_detail;
use super::view;
use crate::app::app_settings;
use crate::container::list::dropdown_trigger;
use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::ContainerRegistry;

/// 渲染镜像仓库弹窗整体界面（根据状态呈现全景详情页或搜索列表）。
pub(crate) fn render_container_hub_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(hub) = state.hub_dialog.as_ref() else {
        return div().into_any_element();
    };

    let searching = hub.searching;
    let search_error = hub.search_error.clone();
    let results = hub.results.clone();
    let selected_reg_id = hub.selected_registry_id.clone();
    let registries = app_settings(cx).container_registries.clone();
    let search_input = hub.search_input.clone();
    let input_val = search_input.read(cx).value().trim().to_string();
    let selected_detail = hub.selected_detail.clone();
    let selected_tags = hub.selected_tags.clone();
    let hub_tags = hub.tags.clone();
    let tags_loading = hub.tags_loading;
    let tags_error = hub.tags_error.clone();
    let custom_tag_input = hub.custom_tag_input.as_ref();
    let copy_toast = hub.copy_toast;

    let current_registry = registries
        .iter()
        .find(|registry| registry.id == selected_reg_id)
        .cloned()
        .unwrap_or_else(ContainerRegistry::docker_hub);
    let current_registry_name = current_registry.name.clone();

    // 1. 顶部 Header。
    let header = view::render_hub_header(&current_registry_name, p);

    // 如果用户正在查看镜像详情视图。
    if let Some(detail_item) = selected_detail {
        let item_name = detail_item.name.clone();
        let active_tag = selected_tags
            .get(&item_name)
            .cloned()
            .unwrap_or_else(|| "latest".to_string());
        let pull_reference = current_registry.qualify_reference(&item_name, &active_tag);

        let view_back = view.clone();
        let view_pull = view.clone();
        let view_copy = view.clone();
        let registry_pull = current_registry.clone();

        let detail_body = render_hub_image_detail(
            super::detail::HubImageDetailParams {
                item: &detail_item,
                active_tag: &active_tag,
                pull_reference: &pull_reference,
                tags: &hub_tags,
                tags_loading,
                tags_error: tags_error.as_deref(),
                custom_tag_input,
                copy_toast,
            },
            move |_, cx| {
                view_back.update(cx, |workspace, cx| workspace.back_to_hub_results(cx));
            },
            move |tag_to_pull, _window, cx| {
                let repository = item_name.clone();
                let registry = registry_pull.clone();
                let tag = tag_to_pull.to_string();
                view_pull.update(cx, move |workspace, cx| {
                    workspace.enqueue_image_pull(repository, tag, registry, cx);
                });
            },
            move |cmd, _, cx| {
                let cmd = cmd.to_string();
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(cmd));
                view_copy.update(cx, |workspace, cx| workspace.mark_hub_copy_toast(cx));
            },
            p,
        );

        let view_close = view.clone();
        let footer = view::render_hub_footer(
            move |_, cx| {
                view_close.update(cx, |workspace, cx| workspace.close_hub_dialog(cx));
            },
            p,
        );

        return v_flex()
            .w_full()
            .gap_3()
            .child(header)
            .child(
                div()
                    .id("container-hub-detail-scroll")
                    .w_full()
                    .max_h(px(480.0))
                    .overflow_y_scroll()
                    .child(detail_body),
            )
            .child(footer)
            .into_any_element();
    }

    // 2. 搜索栏与仓库切换。
    let view_menu = view.clone();
    let reg_menu = dropdown_trigger(
        "container-hub-registry-select",
        current_registry_name.clone(),
        IconName::ChevronDown,
        None,
        move |menu, _, _| {
            let mut menu = menu;
            for registry in &registries {
                let view_item = view_menu.clone();
                let reg_id = registry.id.clone();
                let selected = registry.id == selected_reg_id;
                menu = menu.item(
                    PopupMenuItem::new(registry.name.clone())
                        .checked(selected)
                        .on_click(move |_, _, cx| {
                            let reg_id = reg_id.clone();
                            view_item.update(cx, |workspace, cx| {
                                workspace.select_hub_registry(reg_id, cx);
                            });
                        }),
                );
            }
            menu
        },
    );

    let view_search = view.clone();
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
                    view_search.update(cx, |workspace, cx| workspace.execute_hub_search(cx));
                }),
        );

    // 3. 热门推荐芯片。
    let view_chip = view.clone();
    let chips_bar = view::render_popular_chips(
        move |tag, window, cx| {
            let tag = tag.to_string();
            view_chip.update(cx, move |workspace, cx| {
                workspace.search_hub_for_term(tag, window, cx);
            });
        },
        p,
    );

    // 4. 内容主体分流。
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
        let view_retry = view.clone();
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
                        view_retry.update(cx, |workspace, cx| workspace.execute_hub_search(cx));
                    }),
            )
            .into_any_element()
    } else if results.is_empty() {
        if input_val.is_empty() {
            view::render_hub_welcome_guide(p)
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
            view::render_hub_results_header(results.len(), &current_registry_name, p);
        list = list.child(results_header);

        for item in results {
            let img_name = item.name.clone();
            let registry = current_registry.clone();
            let active_tag = selected_tags
                .get(&img_name)
                .cloned()
                .unwrap_or_else(|| "latest".to_string());

            let view_pull = view.clone();
            let view_tag = view.clone();
            let view_detail = view.clone();
            let item_clone = item.clone();
            let item_for_tag = img_name.clone();
            let item_for_pull = img_name.clone();

            let card = render_hub_result_card(
                &item,
                &active_tag,
                move |tag_to_pull, _window, cx| {
                    let repository = item_for_pull.clone();
                    let registry = registry.clone();
                    let tag = tag_to_pull.to_string();
                    view_pull.update(cx, move |workspace, cx| {
                        workspace.enqueue_image_pull(repository, tag, registry, cx);
                    });
                },
                move |new_tag, _, cx| {
                    let img = item_for_tag.clone();
                    let tag = new_tag.to_string();
                    view_tag.update(cx, move |workspace, cx| {
                        workspace.set_hub_selected_tag(img, tag, cx);
                    });
                },
                move |window, cx| {
                    let it = item_clone.clone();
                    view_detail.update(cx, move |workspace, cx| {
                        workspace.select_hub_image_detail(it, window, cx);
                    });
                },
                p,
            );

            list = list.child(card);
        }

        div()
            .id("container-hub-scroll")
            .w_full()
            .max_h(px(460.0))
            .overflow_y_scroll()
            .child(list)
            .into_any_element()
    };

    // 5. 底部 Footer。
    let view_close = view.clone();
    let footer = view::render_hub_footer(
        move |_, cx| {
            view_close.update(cx, |workspace, cx| workspace.close_hub_dialog(cx));
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
