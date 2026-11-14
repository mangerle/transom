//! 云同步侧边栏：状态、手动操作、冲突处理与活动历史。

mod history;
use history::{
    history_filter_key, render_history_row, short_hash, sync_state_color, sync_state_label,
};

use super::common::*;
use super::history::SyncHistoryEntry;
use super::state::SyncHistoryFilter;
use crate::app::AppShell;
use crate::app::app_settings;
use crate::palette::Palette;
use crate::sidebar::SidebarSection;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use transom_core::{SyncCheckReport, SyncRelation};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

impl AppShell {
    /// 渲染云同步侧边栏；配置入口仍回到设置页。
    pub(crate) fn render_cloud_sync_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let settings = app_settings(cx).sync.clone();
        let state = &self.state.cloud_sync;
        let running = state.is_running();
        let git_active =
            settings.active_channel == transom_core::SyncChannel::Git && settings.git.is_some();
        let configured = git_active || settings.has_endpoint();
        let activated = settings.enabled;
        let enabled = activated && configured;
        let conflict = state.conflict.clone();
        let message = state.message.clone();
        let history = state.history.clone();
        let expanded = state.expanded_history.clone();
        let filter = state.history_filter;
        let limit = state.history_visible_limit;
        let state_label = sync_state_label(
            activated,
            configured,
            conflict.is_some(),
            running,
            state.status,
        );
        let state_color = sync_state_color(
            p,
            activated,
            configured,
            conflict.is_some(),
            running,
            state.status,
        );
        let provider = if git_active {
            "GIT".to_string()
        } else {
            settings.active_endpoint().map_or_else(
                || rust_i18n::t!("sync.not_configured").to_string(),
                |endpoint| endpoint.provider_key().to_uppercase(),
            )
        };
        let baseline = settings.active_baseline();
        let last_success = baseline
            .last_push_at
            .into_iter()
            .chain(baseline.last_pull_at)
            .max()
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| rust_i18n::t!("sync.never").to_string());

        let header = v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .border_color(p.divider(p.sidebar_border))
            .border_b_1()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(8.0)).rounded_full().bg(state_color))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(p.sidebar_foreground)
                                    .child(state_label),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(provider),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sync.last_success", value = last_success)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!(
                        "sync.device_value",
                        name = settings.device_name
                    )),
            )
            .when_some(
                message.filter(|message| !message.trim().is_empty()),
                |this, message| {
                    this.child(
                        div()
                            .text_xs()
                            .line_height(px(17.0))
                            .text_color(p.muted_foreground)
                            .child(message),
                    )
                },
            );

        let mut actions = h_flex()
            .w_full()
            .flex_wrap()
            .gap_1()
            .p_3()
            .border_color(p.divider(p.sidebar_border))
            .border_b_1();
        if git_active {
            actions = actions.child(
                Button::new("cloud-sync-git-now")
                    .small()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::VCS)).small())
                    .label(rust_i18n::t!("sync.git_sync_now").to_string())
                    .disabled(running || !enabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_git_sync(window, cx);
                    })),
            );
        } else {
            actions = actions
                .child(
                    Button::new("cloud-sync-check")
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                        .label(rust_i18n::t!("sync.check").to_string())
                        .disabled(running || !enabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_cloud_sync_check(window, cx);
                        })),
                )
                .child(
                    Button::new("cloud-sync-test")
                        .small()
                        .child(
                            AppIconElement::new(AppIcon::Custom(CustomIcon::CONNECTION_TEST))
                                .small(),
                        )
                        .label(rust_i18n::t!("sync.test").to_string())
                        .disabled(running || !enabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_cloud_sync_test(window, cx);
                        })),
                )
                .child(
                    Button::new("cloud-sync-push")
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::UPLOAD)).small())
                        .label(rust_i18n::t!("sync.push").to_string())
                        .disabled(running || !enabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_cloud_sync_push(window, cx);
                        })),
                )
                .child(
                    Button::new("cloud-sync-pull")
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD)).small())
                        .label(rust_i18n::t!("sync.pull").to_string())
                        .disabled(running || !enabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_cloud_sync_pull(window, cx);
                        })),
                );
        }
        actions = actions.child(
            Button::new("cloud-sync-settings")
                .small()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::SETTINGS)).small())
                .label(rust_i18n::t!("settings.title").to_string())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_settings_modal(None, window, cx);
                })),
        );
        let actions = actions.into_any_element();

        let conflict_card =
            conflict.map(|conflict| render_conflict_card(p, &conflict, running, cx));

        let filter_options = [
            (SyncHistoryFilter::All, rust_i18n::t!("sync.history_all")),
            (
                SyncHistoryFilter::Success,
                rust_i18n::t!("sync.history_success"),
            ),
            (
                SyncHistoryFilter::Failed,
                rust_i18n::t!("sync.history_failed"),
            ),
            (
                SyncHistoryFilter::Conflict,
                rust_i18n::t!("sync.history_conflict"),
            ),
        ];
        let matching: Vec<SyncHistoryEntry> = history
            .into_iter()
            .filter(|entry| filter.matches(entry))
            .collect();
        let matching_count = matching.len();
        let mut rows = v_flex().w_full();
        if matching.is_empty() {
            rows = rows.child(crate::widgets::empty_state(
                AppIcon::Custom(CustomIcon::CLOUD),
                rust_i18n::t!("sync.history_empty").to_string(),
                rust_i18n::t!("sync.history_empty_hint").to_string(),
                p,
            ));
        } else {
            for entry in matching.into_iter().take(limit) {
                let id = entry.id.clone();
                let is_expanded = expanded.contains(&id);
                rows = rows.child(render_history_row(
                    p,
                    entry,
                    is_expanded,
                    cx.listener(move |this, _, _, cx| {
                        this.state.cloud_sync.toggle_history(&id);
                        cx.notify();
                    }),
                ));
            }
            if matching_count > limit {
                rows = rows.child(
                    div().p_2().child(
                        Button::new("cloud-sync-history-more")
                            .ghost()
                            .small()
                            .w_full()
                            .label(rust_i18n::t!("sync.history_more").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.cloud_sync.show_more_history();
                                cx.notify();
                            })),
                    ),
                );
            }
        }

        let filters = h_flex()
            .w_full()
            .flex_wrap()
            .gap_1()
            .px_3()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(p.sidebar_foreground)
                    .child(rust_i18n::t!("sync.history_title").to_string()),
            )
            .children(filter_options.into_iter().map(|(value, label)| {
                Button::new(format!("cloud-sync-filter-{}", history_filter_key(value)))
                    .xsmall()
                    .when(value == filter, |button| button.secondary())
                    .when(value != filter, |button| button.ghost())
                    .label(label.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.cloud_sync.history_filter = value;
                        this.state.cloud_sync.history_visible_limit = 20;
                        cx.notify();
                    }))
                    .into_any_element()
            }));

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .child(header)
            .child(actions)
            .when_some(conflict_card, |this, card| this.child(card))
            .when(!enabled, |this| {
                this.child(
                    div().p_3().child(
                        Button::new("cloud-sync-open-settings-disabled")
                            .small()
                            .label(rust_i18n::t!("sync.configure_now").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings_modal(None, window, cx);
                            })),
                    ),
                )
            })
            .child(filters)
            .child(
                div()
                    .id("cloud-sync-history-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
            .into_any_element()
    }

    /// 显示云同步侧边栏；活动栏点击统一走这里，保持收起再展开行为一致。
    pub(crate) fn toggle_cloud_sync_sidebar(&mut self, cx: &mut Context<Self>) {
        self.state.sidebar.toggle_section(SidebarSection::CloudSync);
        cx.notify();
    }
}

fn render_conflict_card(
    p: &Palette,
    conflict: &SyncCheckReport,
    running: bool,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let remote = conflict.remote_summary.as_ref().map_or_else(
        || rust_i18n::t!("sync.remote_absent").to_string(),
        |summary| {
            rust_i18n::t!(
                "sync.remote_summary",
                sessions = summary.sessions,
                folders = summary.folders
            )
            .to_string()
        },
    );
    let title = if conflict.relation == SyncRelation::RemoteMissing {
        rust_i18n::t!("sync.conflict_remote_missing").to_string()
    } else {
        rust_i18n::t!("sync.conflict_diverged").to_string()
    };
    v_flex()
        .m_2()
        .rounded_md()
        .border_1()
        .border_color(p.warning)
        .bg(p.sidebar)
        .p_3()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(p.warning)
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!(
                    "sync.conflict_local_summary",
                    sessions = conflict.local_summary.sessions,
                    folders = conflict.local_summary.folders
                )),
        )
        .child(div().text_xs().text_color(p.muted_foreground).child(remote))
        .child(
            div()
                .font_family("monospace")
                .text_xs()
                .text_color(p.muted_foreground)
                .child(format!(
                    "{} / {}",
                    short_hash(&conflict.local_hash),
                    conflict
                        .remote_hash
                        .as_deref()
                        .map(short_hash)
                        .unwrap_or_else(|| "--".to_string())
                )),
        )
        .child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("cloud-sync-force-pull")
                        .small()
                        .label(rust_i18n::t!("sync.download_remote_version").to_string())
                        .disabled(running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_cloud_sync_force_pull(cx);
                        })),
                )
                .child(
                    Button::new("cloud-sync-force-push")
                        .small()
                        .primary()
                        .label(rust_i18n::t!("sync.upload_local_version").to_string())
                        .disabled(running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_cloud_sync_force_push(cx);
                        })),
                ),
        )
        .into_any_element()
}
