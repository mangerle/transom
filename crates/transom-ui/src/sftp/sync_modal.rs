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
    pub(crate) fn render_sftp_sync_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
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
                        .child("正在扫描并比较本地与远端目录，请稍候..."),
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
                        .label("重新扫描")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_open_sync_modal(tab_id, cx);
                        })),
                )
                .into_any_element()
        } else if entries.is_empty() {
            v_flex()
                .h(px(360.0))
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_sm().child("两个目录内容一致，无需同步"))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format!("本次扫描排除 {skipped} 个条目")),
                )
                .into_any_element()
        } else {
            let row_sizes = Rc::new(vec![size(px(0.0), px(32.0)); entries.len()]);
            let entries_rc = Rc::new(entries);
            let selected_rc = Rc::new(selected);
            // 虚拟列表闭包单独持有一份调色板：外层渲染继续用原值，互不移动。
            let list_palette = p.clone();
            let list = v_virtual_list(
                cx.entity().clone(),
                format!("sftp-sync-virtual-list-{tab_id}"),
                row_sizes,
                move |_this, range, _window, cx| {
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
                                        .on_change(cx.listener(move |this, &checked, _, cx| {
                                            this.sftp_toggle_sync_entry(index, checked, cx);
                                        })),
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
                        .child(div().flex_1().child("相对路径"))
                        .child(div().w(px(72.0)).child("差异"))
                        .child(div().w(px(132.0)).child("本地"))
                        .child(div().w(px(132.0)).child("远端")),
                )
                .child(
                    v_flex()
                        .h(px(330.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(p.border)
                        .rounded_md()
                        .child(list),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format!(
                            "差异 {diff_count} 项：待上传 {upload_count}，待下载 {download_count}，类型冲突 {conflict_count}；扫描排除 {skipped} 项"
                        )),
                )
                .into_any_element()
        };

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-sync")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sftp_modals.sync = None;
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    })),
            )
            .when(can_start, |this| {
                this.child(
                    Button::new("select-all-sftp-sync")
                        .ghost()
                        .xsmall()
                        .label("全选")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sftp_select_all_sync(true, cx);
                        })),
                )
                .child(
                    Button::new("select-none-sftp-sync")
                        .ghost()
                        .xsmall()
                        .label("全不选")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sftp_select_all_sync(false, cx);
                        })),
                )
                .child(
                    Button::new("start-sftp-sync")
                        .primary()
                        .icon(IconName::GitCompareArrows)
                        .label(format!("开始同步 ({selected_count})"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_sftp_sync(window, cx);
                        })),
                )
            });

        let dialog = Self::dialog_frame(
            "sftp-sync-modal",
            "目录对比与增量同步",
            Some(format!("本地: {local_root}  |  远端: {remote_root}")),
            content,
            Some(footer),
            px(760.0),
            p,
        );

        Self::overlay(dialog, p)
    }
}

fn status_presentation(status: DirDiffStatus, p: &Palette) -> (&'static str, Hsla) {
    match status {
        DirDiffStatus::LocalOnly => ("仅本地", p.success),
        DirDiffStatus::RemoteOnly => ("仅远端", p.sidebar_accent_foreground),
        DirDiffStatus::LocalNewer => ("本地较新", p.success),
        DirDiffStatus::RemoteNewer => ("远端较新", p.sidebar_accent_foreground),
        DirDiffStatus::TypeMismatch => ("类型冲突", p.danger),
    }
}

fn side_text(meta: Option<&transom_core::DirEntryMeta>) -> String {
    let Some(meta) = meta else {
        return "—".to_string();
    };
    if meta.is_dir {
        return "目录".to_string();
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
