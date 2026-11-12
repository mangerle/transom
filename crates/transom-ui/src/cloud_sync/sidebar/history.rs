//! 云同步历史行与状态颜色辅助渲染。

use super::super::history::{SyncHistoryEntry, SyncHistoryStatus};
use super::super::state::SyncHistoryFilter;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::SyncOperation;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(super) fn render_history_row(
    p: &Palette,
    entry: SyncHistoryEntry,
    expanded: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let icon = match entry.status {
        SyncHistoryStatus::Success => AppIcon::Ui(UiIcon::CHECK),
        SyncHistoryStatus::Conflict => AppIcon::Ui(UiIcon::WARNING),
        SyncHistoryStatus::Failed => AppIcon::Ui(UiIcon::ERROR),
        SyncHistoryStatus::Running => AppIcon::Ui(UiIcon::REFRESH),
        SyncHistoryStatus::Idle => AppIcon::Ui(UiIcon::HISTORY),
    };
    let color = match entry.status {
        SyncHistoryStatus::Success => p.success,
        SyncHistoryStatus::Conflict => p.warning,
        SyncHistoryStatus::Failed => p.danger,
        SyncHistoryStatus::Running => p.primary,
        SyncHistoryStatus::Idle => p.muted_foreground,
    };
    let time = chrono::DateTime::from_timestamp_millis(entry.at_ms)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "--".to_string());
    v_flex()
        .w_full()
        .gap_1()
        .px_3()
        .py_2()
        .border_color(p.divider(p.sidebar_border))
        .border_b_1()
        .child(
            h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .mt(px(2.0))
                        .flex_none()
                        .child(AppIconElement::new(icon).xsmall().text_color(color)),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .gap_1()
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .child(history_title(&entry)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(time),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(entry.message.clone()),
                        )
                        .when(expanded, |this| {
                            this.child(div().text_xs().text_color(p.muted_foreground).child(
                                format!(
                                    "{} · {} · {}ms · {}",
                                    entry.provider,
                                    entry.trigger.key(),
                                    entry.duration_ms,
                                    format_bytes(entry.bytes)
                                ),
                            ))
                        })
                        .child(
                            Button::new(format!("cloud-sync-history-toggle-{}", entry.id))
                                .xsmall()
                                .ghost()
                                .label(if expanded {
                                    rust_i18n::t!("sync.history_collapse").to_string()
                                } else {
                                    rust_i18n::t!("sync.history_expand").to_string()
                                })
                                .on_click(on_toggle),
                        ),
                ),
        )
        .into_any_element()
}

fn history_title(entry: &SyncHistoryEntry) -> String {
    match entry.operation {
        SyncOperation::Check => rust_i18n::t!("sync.operation_check").to_string(),
        SyncOperation::TestConnection => rust_i18n::t!("sync.operation_test").to_string(),
        SyncOperation::Push => rust_i18n::t!("sync.operation_push").to_string(),
        SyncOperation::Pull => rust_i18n::t!("sync.operation_pull").to_string(),
        SyncOperation::Git => rust_i18n::t!("sync.operation_git").to_string(),
    }
}

pub(super) fn history_filter_key(filter: SyncHistoryFilter) -> &'static str {
    match filter {
        SyncHistoryFilter::All => "all",
        SyncHistoryFilter::Success => "success",
        SyncHistoryFilter::Failed => "failed",
        SyncHistoryFilter::Conflict => "conflict",
    }
}

pub(super) fn sync_state_label(
    activated: bool,
    configured: bool,
    has_conflict: bool,
    running: bool,
    last_status: SyncHistoryStatus,
) -> String {
    if !configured {
        return rust_i18n::t!("sync.not_configured").to_string();
    }
    if !activated {
        return rust_i18n::t!("sync.state_disabled").to_string();
    }
    if has_conflict {
        return rust_i18n::t!("sync.state_conflict").to_string();
    }
    if running {
        return rust_i18n::t!("sync.state_running").to_string();
    }
    match last_status {
        SyncHistoryStatus::Success => rust_i18n::t!("sync.state_success").to_string(),
        SyncHistoryStatus::Failed => rust_i18n::t!("sync.state_failed").to_string(),
        SyncHistoryStatus::Conflict => rust_i18n::t!("sync.state_conflict").to_string(),
        SyncHistoryStatus::Running => rust_i18n::t!("sync.state_running").to_string(),
        SyncHistoryStatus::Idle => rust_i18n::t!("sync.state_idle").to_string(),
    }
}

pub(super) fn sync_state_color(
    p: &Palette,
    activated: bool,
    configured: bool,
    has_conflict: bool,
    running: bool,
    last_status: SyncHistoryStatus,
) -> gpui::Hsla {
    if !configured || !activated {
        return p.muted_foreground;
    }
    if has_conflict {
        return p.warning;
    }
    if running {
        return p.primary;
    }
    match last_status {
        SyncHistoryStatus::Success => p.success,
        SyncHistoryStatus::Failed => p.danger,
        SyncHistoryStatus::Conflict => p.warning,
        SyncHistoryStatus::Running => p.primary,
        SyncHistoryStatus::Idle => p.muted_foreground,
    }
}

pub(super) fn short_hash(value: &str) -> String {
    value.chars().take(12).collect()
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "--".to_string();
    }
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KiB", (bytes / 1024).max(1))
    }
}
