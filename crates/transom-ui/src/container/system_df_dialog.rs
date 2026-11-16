//! 容器引擎全局磁盘占用看板与联合深度清理弹窗。
//!
//! 动作归属（Why）：打开、关闭、刷新与联合清理只读写本标签页状态并投递命令，
//! 全部落在 [`ContainerWorkspace`] 上；弹窗只负责读取状态快照并直接回调实体。

use super::*;
use crate::format::format_size;
use crate::palette::Palette;
use crate::widgets::action_button;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::*;
use transom_ui_icons::UiIcon;

impl ContainerWorkspace {
    /// 打开全局存储分析与空间体检弹窗。
    pub(crate) fn open_system_df_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.modals.system_df_dialog_open = true;
        let needs_fetch = self.state.system_df.is_none() && !self.state.system_df_loading;
        if needs_fetch {
            self.refresh_system_df(cx);
        }
        cx.notify();
    }

    /// 关闭全局存储分析弹窗。
    pub(crate) fn close_system_df_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.modals.system_df_dialog_open = false;
        cx.notify();
    }

    /// 重新分析拉取全局磁盘占用。
    pub(crate) fn refresh_system_df(&mut self, cx: &mut Context<Self>) {
        let request_id = ContainerRequestId::new();
        let previous = self.state.system_df_request.replace(request_id);
        self.state.system_df_loading = true;
        if let Some(previous) = previous {
            self.cancel_container_request(previous);
        }
        log::info!("正在查询全局系统磁盘占用，请求: {request_id}");
        self.send_tracked(
            request_id,
            ContainerCommand::SystemDf {
                tab_id: self.tab_id,
                request_id,
            },
        );
        cx.notify();
    }

    /// 触发系统级联合深度清理（System Prune）。
    pub(crate) fn submit_system_prune(&mut self, cx: &mut Context<Self>) {
        if self.state.system_prune_running {
            return;
        }
        self.state.system_prune_running = true;
        let all = self.state.system_prune_all;
        let volumes = self.state.system_prune_volumes;
        let request_id = ContainerRequestId::new();
        log::info!(
            "正在发起系统级联合清理，全量镜像: {all}，包含数据卷: {volumes}，请求: {request_id}"
        );
        self.send_tracked(
            request_id,
            ContainerCommand::SystemPrune {
                tab_id: self.tab_id,
                request_id,
                all,
                volumes,
            },
        );
        cx.notify();
    }

    /// 切换「清理全部未使用镜像」选项。
    pub(crate) fn toggle_system_prune_all(&mut self, cx: &mut Context<Self>) {
        self.state.system_prune_all = !self.state.system_prune_all;
        cx.notify();
    }

    /// 切换「包含未使用数据卷」选项。
    pub(crate) fn toggle_system_prune_volumes(&mut self, cx: &mut Context<Self>) {
        self.state.system_prune_volumes = !self.state.system_prune_volumes;
        cx.notify();
    }
}

/// 渲染存储体检看板弹窗体。
///
/// 设计原理（Why）：看板数据、加载/清理状态与两个清理选项都归工作区实体，
/// 渲染只读快照；刷新、关闭、选项切换与清理回调直接 `update` 实体。
pub(crate) fn render_container_system_df_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let loading = state.system_df_loading;
    let prune_running = state.system_prune_running;
    let usage = state.system_df.clone().unwrap_or_default();
    let has_snapshot = state.system_df.is_some();
    let content = v_flex()
        .gap_3()
        .child(df_summary_banner(&usage, loading, has_snapshot, p))
        .child(super::system_df_view::render_stacked_usage_bar(&usage, p))
        .child(super::system_df_view::render_df_details_list(&usage, p))
        .child(df_options_section(
            view,
            state.system_prune_all,
            state.system_prune_volumes,
            p,
        ))
        .child(df_actions_bar(
            view,
            loading,
            prune_running,
            usage.total_reclaimable > 0 || state.system_prune_all,
        ));
    crate::widgets::dialog_body(
        "container-system-df-modal",
        rust_i18n::t!("container.system_df.title").to_string(),
        Some(rust_i18n::t!("container.system_df.subtitle").to_string()),
        content,
        None::<AnyElement>,
    )
}

fn df_summary_banner(
    usage: &transom_core::ContainerSystemDiskUsage,
    loading: bool,
    has_snapshot: bool,
    p: &Palette,
) -> AnyElement {
    let total_size_str = format_size(usage.total_size);
    let reclaim_size_str = format_size(usage.total_reclaimable);
    h_flex()
        .justify_between()
        .items_center()
        .p_4()
        .rounded(p.radius.md)
        .bg(p.secondary)
        .border_1()
        .border_color(p.border)
        .child(df_banner_cell(
            "container.system_df.total_used",
            p.foreground,
            banner_value(&total_size_str, loading && !has_snapshot),
            p,
        ))
        .child(
            v_flex()
                .gap_1()
                .items_end()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.system_df.total_reclaimable").to_string()),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(if usage.total_reclaimable > 0 {
                            p.warning
                        } else {
                            p.muted_foreground
                        })
                        .child(banner_value(&reclaim_size_str, loading && !has_snapshot)),
                ),
        )
        .into_any_element()
}

fn df_banner_cell(
    label_key: &'static str,
    value_color: gpui_kit::Hsla,
    value: AnyElement,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!(label_key).to_string()),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(value_color)
                .child(value),
        )
        .into_any_element()
}

fn banner_value(text: &str, loading: bool) -> AnyElement {
    if loading {
        Spinner::new().small().into_any_element()
    } else {
        div().child(text.to_string()).into_any_element()
    }
}

fn df_options_section(
    view: &Entity<ContainerWorkspace>,
    prune_all: bool,
    prune_volumes: bool,
    p: &Palette,
) -> AnyElement {
    let p = p.clone();
    v_flex()
        .gap_2()
        .p_3()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.1))
        .border_1()
        .border_color(p.border)
        .child(df_prune_all_row(view, prune_all, &p))
        .child(df_prune_volumes_row(view, prune_volumes, &p))
        .into_any_element()
}

fn df_prune_all_row(view: &Entity<ContainerWorkspace>, enabled: bool, p: &Palette) -> AnyElement {
    let view = view.clone();
    h_flex()
        .items_center()
        .justify_between()
        .child(df_option_text(
            "container.system_df.opt_prune_all",
            "container.system_df.opt_prune_all_hint",
            p,
        ))
        .child(
            Button::new("opt-prune-all-toggle")
                .xsmall()
                .ghost()
                .selected(enabled)
                .label(df_option_label(enabled))
                .on_click(move |_, _, cx| {
                    view.update(cx, |workspace, cx| workspace.toggle_system_prune_all(cx));
                }),
        )
        .into_any_element()
}

fn df_prune_volumes_row(
    view: &Entity<ContainerWorkspace>,
    enabled: bool,
    p: &Palette,
) -> AnyElement {
    let view = view.clone();
    h_flex()
        .items_center()
        .justify_between()
        .child(df_option_text(
            "container.system_df.opt_prune_volumes",
            "container.system_df.opt_prune_volumes_hint",
            p,
        ))
        .child(
            Button::new("opt-prune-volumes-toggle")
                .xsmall()
                .ghost()
                .selected(enabled)
                .label(df_option_label(enabled))
                .on_click(move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.toggle_system_prune_volumes(cx)
                    });
                }),
        )
        .into_any_element()
}

fn df_option_text(label_key: &'static str, hint_key: &'static str, p: &Palette) -> AnyElement {
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(rust_i18n::t!(label_key).to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!(hint_key).to_string()),
        )
        .into_any_element()
}

fn df_option_label(enabled: bool) -> String {
    if enabled {
        rust_i18n::t!("container.system_df.opt_enabled").to_string()
    } else {
        rust_i18n::t!("container.system_df.opt_disabled").to_string()
    }
}

fn df_actions_bar(
    view: &Entity<ContainerWorkspace>,
    loading: bool,
    prune_running: bool,
    can_prune: bool,
) -> AnyElement {
    let refresh_view = view.clone();
    let close_view = view.clone();
    let submit_view = view.clone();
    h_flex()
        .justify_between()
        .items_center()
        .pt_2()
        .child(
            action_button(
                "system-df-refresh",
                UiIcon::REFRESH,
                rust_i18n::t!("container.system_df.btn_refresh").to_string(),
            )
            .ghost()
            .disabled(loading || prune_running)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |workspace, cx| workspace.refresh_system_df(cx));
            }),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("system-df-close")
                        .small()
                        .ghost()
                        .label(rust_i18n::t!("common.close").to_string())
                        .on_click(move |_, _, cx| {
                            close_view
                                .update(cx, |workspace, cx| workspace.close_system_df_dialog(cx));
                        }),
                )
                .child(
                    action_button(
                        "system-df-submit-prune",
                        UiIcon::DELETE,
                        if prune_running {
                            rust_i18n::t!("container.system_df.pruning").to_string()
                        } else {
                            rust_i18n::t!("container.system_df.btn_prune").to_string()
                        },
                    )
                    .danger()
                    .disabled(loading || prune_running || !can_prune)
                    .on_click(move |_, _, cx| {
                        submit_view.update(cx, |workspace, cx| workspace.submit_system_prune(cx));
                    }),
                ),
        )
        .into_any_element()
}
