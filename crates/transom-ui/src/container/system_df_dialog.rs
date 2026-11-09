//! 容器引擎全局磁盘占用看板与联合深度清理弹窗。
//!
//! 动作归属（Why）：打开、关闭、刷新与联合清理只读写本标签页状态并投递命令，
//! 全部落在 [`ContainerWorkspace`] 上；弹窗只负责读取状态快照并直接回调实体。

use super::*;
use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::*;

impl ContainerWorkspace {
    /// 打开全局存储分析与空间体检弹窗。
    pub(crate) fn open_system_df_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.system_df_dialog_open = true;
        let needs_fetch = self.state.system_df.is_none() && !self.state.system_df_loading;
        if needs_fetch {
            self.refresh_system_df(cx);
        }
        cx.notify();
    }

    /// 关闭全局存储分析弹窗。
    pub(crate) fn close_system_df_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.system_df_dialog_open = false;
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
    let prune_all = state.system_prune_all;
    let prune_volumes = state.system_prune_volumes;

    let total_size_str = format_size(usage.total_size);
    let reclaim_size_str = format_size(usage.total_reclaimable);

    // 顶部总览横幅。
    let summary_banner = h_flex()
        .justify_between()
        .items_center()
        .p_4()
        .rounded(p.radius.md)
        .bg(p.secondary)
        .border_1()
        .border_color(p.border)
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.system_df.total_used").to_string()),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.foreground)
                        .child(if loading && state.system_df.is_none() {
                            "...".to_string()
                        } else {
                            total_size_str
                        }),
                ),
        )
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
                        .child(if loading && state.system_df.is_none() {
                            "...".to_string()
                        } else {
                            reclaim_size_str
                        }),
                ),
        );

    // 堆叠占用比例条与分类明细看板。
    let stacked_bar = super::system_df_view::render_stacked_usage_bar(&usage, p);
    let details_list = super::system_df_view::render_df_details_list(&usage, p);

    // 清理选项控制。
    let options_section = v_flex()
        .gap_2()
        .p_3()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.1))
        .border_1()
        .border_color(p.border)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div().text_sm().font_weight(FontWeight::MEDIUM).child(
                                rust_i18n::t!("container.system_df.opt_prune_all").to_string(),
                            ),
                        )
                        .child(div().text_xs().text_color(p.muted_foreground).child(
                            rust_i18n::t!("container.system_df.opt_prune_all_hint").to_string(),
                        )),
                )
                .child(
                    Button::new("opt-prune-all-toggle")
                        .xsmall()
                        .ghost()
                        .selected(prune_all)
                        .label(if prune_all {
                            rust_i18n::t!("container.system_df.opt_enabled").to_string()
                        } else {
                            rust_i18n::t!("container.system_df.opt_disabled").to_string()
                        })
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |workspace, cx| {
                                    workspace.toggle_system_prune_all(cx)
                                });
                            }
                        }),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(
                            rust_i18n::t!("container.system_df.opt_prune_volumes").to_string(),
                        ))
                        .child(div().text_xs().text_color(p.muted_foreground).child(
                            rust_i18n::t!("container.system_df.opt_prune_volumes_hint").to_string(),
                        )),
                )
                .child(
                    Button::new("opt-prune-volumes-toggle")
                        .xsmall()
                        .ghost()
                        .selected(prune_volumes)
                        .label(if prune_volumes {
                            rust_i18n::t!("container.system_df.opt_enabled").to_string()
                        } else {
                            rust_i18n::t!("container.system_df.opt_disabled").to_string()
                        })
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |workspace, cx| {
                                    workspace.toggle_system_prune_volumes(cx)
                                });
                            }
                        }),
                ),
        );

    // 底部动作条。
    let actions_bar = h_flex()
        .justify_between()
        .items_center()
        .pt_2()
        .child(
            Button::new("system-df-refresh")
                .xsmall()
                .ghost()
                .icon(IconName::RefreshCw)
                .label(rust_i18n::t!("container.system_df.btn_refresh").to_string())
                .disabled(loading || prune_running)
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| workspace.refresh_system_df(cx));
                    }
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
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |workspace, cx| {
                                    workspace.close_system_df_dialog(cx)
                                });
                            }
                        }),
                )
                .child(
                    Button::new("system-df-submit-prune")
                        .small()
                        .danger()
                        .icon(IconName::Trash)
                        .label(if prune_running {
                            rust_i18n::t!("container.system_df.pruning").to_string()
                        } else {
                            rust_i18n::t!("container.system_df.btn_prune").to_string()
                        })
                        .disabled(
                            loading
                                || prune_running
                                || (usage.total_reclaimable == 0 && !prune_all),
                        )
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |workspace, cx| workspace.submit_system_prune(cx));
                            }
                        }),
                ),
        );

    let content = v_flex()
        .gap_3()
        .child(summary_banner)
        .child(stacked_bar)
        .child(details_list)
        .child(options_section)
        .child(actions_bar);

    crate::widgets::dialog_body(
        "container-system-df-modal",
        rust_i18n::t!("container.system_df.title").to_string(),
        Some(rust_i18n::t!("container.system_df.subtitle").to_string()),
        content,
        None::<AnyElement>,
    )
}
