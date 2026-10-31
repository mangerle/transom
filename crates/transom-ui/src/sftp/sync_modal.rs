//! 目录对比与增量同步弹窗视图。

use super::{format_size, format_unix_time};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Sizable, v_virtual_list};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::DirDiffStatus;

impl AppShell {
    pub(crate) fn render_sftp_sync_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.sync.as_ref() else {
            return div().into_any_element();
        };
        let tab_id = modal.tab_id;
        let scanning = modal.scanning;
        let error = modal.error.clone();
        let skipped = modal.skipped;
        let local_root = modal.local_root.display().to_string();
        let remote_root = modal.remote_root.clone();
        let entries = modal.entries.clone();
        let selected = modal.selected.clone();
        let scroll_handle = modal.scroll_handle.clone();

        let upload_count = entries
            .iter()
            .zip(selected.iter())
            .filter(|(entry, checked)| {
                **checked
                    && entry.status.suggested_direction()
                        == Some(transom_core::TransferDirection::Upload)
            })
            .count();
        let download_count = entries
            .iter()
            .zip(selected.iter())
            .filter(|(entry, checked)| {
                **checked
                    && entry.status.suggested_direction()
                        == Some(transom_core::TransferDirection::Download)
            })
            .count();
        let conflict_count = entries
            .iter()
            .filter(|entry| entry.status == DirDiffStatus::TypeMismatch)
            .count();
        let selected_count = selected.iter().filter(|checked| **checked).count();
        let diff_count = entries.len();
        let has_error = error.is_some();
        let can_start = !scanning && !has_error && diff_count > 0;

        let content = if scanning {
            v_flex()
                .h(px(360.0))
                .items_center()
                .justify_center()
                .gap_3()
                .child(Progress::new("sftp-sync-scan-progress").loading(true))
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("sftp.sync.scanning").to_string()),
                )
                .into_any_element()
        } else if let Some(err) = error {
            v_flex()
                .h(px(360.0))
                .items_center()
                .justify_center()
                .gap_3()
                .child(div().text_sm().text_color(p.danger).child(err))
                .child(
                    Button::new("retry-sftp-sync-scan")
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .label(rust_i18n::t!("sftp.sync.rescan").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            move |this, _: &ClickEvent, _, cx| {
                                this.sftp_open_sync_modal(tab_id, cx);
                            },
                        )),
                )
                .into_any_element()
        } else if entries.is_empty() {
            v_flex()
                .h(px(360.0))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("sftp.sync.identical").to_string()),
                )
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!("sftp.sync.skipped_count", count = skipped).to_string(),
                    ),
                )
                .into_any_element()
        } else {
            let row_sizes = Rc::new(vec![size(px(0.0), px(32.0)); entries.len()]);
            let entries_rc = Rc::new(entries);
            let selected_rc = Rc::new(selected);
            // 虚拟列表闭包单独持有一份调色板与外壳句柄：这两样要跨到
            // 'static 的渲染闭包里，方法参数的借用活不到那时候。
            let list_palette = p.clone();
            let list_app = app.clone();
            let list = v_virtual_list(
                app.clone(),
                format!("sftp-sync-virtual-list-{tab_id}"),
                row_sizes,
                move |_this, range, _window, _cx| {
                    range
                        .filter_map(|index| {
                            let entry = entries_rc.get(index)?;
                            let checked = selected_rc.get(index).copied().unwrap_or(false);
                            let selectable = entry.status != DirDiffStatus::TypeMismatch;
                            let (status_label, status_color) =
                                status_presentation(entry.status, &list_palette);
                            let local_text = side_text(entry.local.as_ref());
                            let remote_text = side_text(entry.remote.as_ref());
                            let mut row = h_flex()
                                .id(ElementId::Name(format!("sftp-sync-row-{index}").into()))
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .text_xs()
                                .border_b_1()
                                .border_color(list_palette.sidebar_border);
                            if selectable {
                                row = row.child(
                                    Checkbox::new(format!("sftp-sync-check-{index}"))
                                        .checked(checked)
                                        .on_change(crate::dialog_host::on_event(
                                            &list_app,
                                            move |this, checked: &bool, _, cx| {
                                                this.sftp_toggle_sync_entry(index, *checked, cx);
                                            },
                                        )),
                                );
                            } else {
                                row = row.child(div().w(px(16.0)));
                            }
                            Some(
                                row.child(
                                    div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .child(entry.relative_path.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(72.0))
                                        .text_color(status_color)
                                        .child(status_label),
                                )
                                .child(
                                    div()
                                        .w(px(132.0))
                                        .text_color(list_palette.muted_foreground)
                                        .child(local_text),
                                )
                                .child(
                                    div()
                                        .w(px(132.0))
                                        .text_color(list_palette.muted_foreground)
                                        .child(remote_text),
                                )
                                .into_any_element(),
                            )
                        })
                        .collect()
                },
            )
            .track_scroll(&scroll_handle)
            .into_any_element();

            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(
                            div()
                                .flex_1()
                                .child(rust_i18n::t!("sftp.sync.column_relative_path").to_string()),
                        )
                        .child(
                            div()
                                .w(px(72.0))
                                .child(rust_i18n::t!("sftp.sync.column_status").to_string()),
                        )
                        .child(
                            div()
                                .w(px(132.0))
                                .child(rust_i18n::t!("sftp.sync.column_local").to_string()),
                        )
                        .child(
                            div()
                                .w(px(132.0))
                                .child(rust_i18n::t!("sftp.sync.column_remote").to_string()),
                        ),
                )
                .child(
                    v_flex()
                        .h(px(330.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(p.border)
                        .rounded(p.radius.md)
                        .child(list),
                )
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!(
                            "sftp.sync.summary",
                            diff = diff_count,
                            upload = upload_count,
                            download = download_count,
                            conflict = conflict_count,
                            skipped = skipped
                        )
                        .to_string(),
                    ),
                )
                .into_any_element()
        };

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-sync")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, window, cx| {
                            this.sftp_modals.sync = None;
                            this.focus_active_sftp_panel(window, cx);
                            cx.notify();
                        },
                    )),
            )
            .when(can_start, |this| {
                this.child(
                    Button::new("select-all-sftp-sync")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("sftp.sync.select_all").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, _, cx| {
                                this.sftp_select_all_sync(true, cx);
                            },
                        )),
                )
                .child(
                    Button::new("select-none-sftp-sync")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("sftp.sync.select_none").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, _, cx| {
                                this.sftp_select_all_sync(false, cx);
                            },
                        )),
                )
                .child(
                    Button::new("start-sftp-sync")
                        .primary()
                        .icon(IconName::GitCompareArrows)
                        .label(rust_i18n::t!("sftp.sync.start", count = selected_count).to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, window, cx| {
                                this.start_sftp_sync(window, cx);
                            },
                        )),
                )
            });

        Self::dialog_body(
            "sftp-sync-modal",
            rust_i18n::t!("sftp.sync.title").to_string(),
            Some(
                rust_i18n::t!(
                    "sftp.sync.subtitle",
                    local = local_root.as_str(),
                    remote = remote_root.as_str()
                )
                .to_string(),
            ),
            content,
            Some(footer),
        )
    }
}

fn status_presentation(status: DirDiffStatus, p: &Palette) -> (String, Hsla) {
    match status {
        DirDiffStatus::LocalOnly => (
            rust_i18n::t!("sftp.sync.status_local_only").to_string(),
            p.success,
        ),
        DirDiffStatus::RemoteOnly => (
            rust_i18n::t!("sftp.sync.status_remote_only").to_string(),
            p.sidebar_accent_foreground,
        ),
        DirDiffStatus::LocalNewer => (
            rust_i18n::t!("sftp.sync.status_local_newer").to_string(),
            p.success,
        ),
        DirDiffStatus::RemoteNewer => (
            rust_i18n::t!("sftp.sync.status_remote_newer").to_string(),
            p.sidebar_accent_foreground,
        ),
        DirDiffStatus::TypeMismatch => (
            rust_i18n::t!("sftp.sync.status_type_mismatch").to_string(),
            p.danger,
        ),
    }
}

fn side_text(meta: Option<&transom_core::DirEntryMeta>) -> String {
    let Some(meta) = meta else {
        return "—".to_string();
    };
    if meta.is_dir {
        return rust_i18n::t!("sftp.sync.side_directory").to_string();
    }
    let size = format_size(meta.size);
    match meta.mtime {
        Some(seconds) => format!("{size} {}", short_time(seconds)),
        None => size,
    }
}

fn short_time(seconds: u32) -> String {
    let full = format_unix_time(seconds);
    full.get(5..16).map(str::to_string).unwrap_or(full)
}
