//! 目录对比与增量同步弹窗视图实体。

use super::{SftpSyncModalState, format_size, format_unix_time};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::v_virtual_list;
use gpui_kit::*;
use std::path::PathBuf;
use std::rc::Rc;
use transom_core::{ConnectionId, DirComparison, DirDiffEntry, DirDiffStatus, TabId, TransferId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

mod sync_footer;
use sync_footer::{sync_footer, sync_select_all_state};

/// 目录对比与同步弹窗事件。
pub(crate) enum SftpSyncModalEvent {
    Rescan(TabId),
    Start {
        tab_id: TabId,
        local_root: PathBuf,
        remote_root: String,
        entries: Vec<DirDiffEntry>,
    },
    Cancel,
}

/// 目录对比与增量同步弹窗实体视图。
pub(crate) struct SftpSyncModalView {
    pub(crate) state: SftpSyncModalState,
}

impl EventEmitter<SftpSyncModalEvent> for SftpSyncModalView {}

impl Render for SftpSyncModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_body(cx, &p)
    }
}

impl SftpSyncModalView {
    pub(crate) fn new(state: SftpSyncModalState) -> Self {
        Self { state }
    }

    /// 目录对比完成：校验连接与操作匹配后回填扫描结果。
    pub(crate) fn apply_dir_compared(
        &mut self,
        connection_id: ConnectionId,
        op_id: TransferId,
        comparison: &DirComparison,
        cx: &mut Context<Self>,
    ) {
        if self.state.op_id == op_id && self.state.connection_id == connection_id {
            self.state.scanning = false;
            self.state.skipped = comparison.skipped;
            self.state.selected = comparison
                .entries
                .iter()
                .map(|entry| entry.sync_item().is_some())
                .collect();
            self.state.entries = comparison.entries.clone();
            self.state.error = None;
            cx.notify();
        }
    }

    pub(crate) fn toggle_entry(&mut self, index: usize, checked: bool, cx: &mut Context<Self>) {
        if let Some(slot) = self.state.selected.get_mut(index) {
            *slot = checked;
            cx.notify();
        }
    }

    pub(crate) fn select_all(&mut self, checked: bool, cx: &mut Context<Self>) {
        for (entry, slot) in self
            .state
            .entries
            .iter()
            .zip(self.state.selected.iter_mut())
        {
            if entry.status != DirDiffStatus::TypeMismatch {
                *slot = checked;
            }
        }
        cx.notify();
    }

    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        if self.state.scanning || self.state.error.is_some() {
            return;
        }
        let selected_entries: Vec<DirDiffEntry> = self
            .state
            .entries
            .iter()
            .zip(self.state.selected.iter())
            .filter(|(entry, checked)| **checked && entry.status != DirDiffStatus::TypeMismatch)
            .map(|(entry, _)| entry.clone())
            .collect();
        cx.emit(SftpSyncModalEvent::Start {
            tab_id: self.state.tab_id,
            local_root: self.state.local_root.clone(),
            remote_root: self.state.remote_root.clone(),
            entries: selected_entries,
        });
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(SftpSyncModalEvent::Cancel);
    }

    fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let scanning = self.state.scanning;
        let error = self.state.error.as_ref();
        let local_root = self.state.local_root.display().to_string();
        let remote_root = self.state.remote_root.as_str();
        let selected_count = self
            .state
            .entries
            .iter()
            .zip(self.state.selected.iter())
            .filter(|(entry, checked)| **checked && entry.status != DirDiffStatus::TypeMismatch)
            .count();
        let selection_state = sync_select_all_state(&self.state.entries, &self.state.selected);
        let diff_count = self.state.entries.len();
        let can_start = !scanning && error.is_none() && diff_count > 0;

        let content = match sync_status_panel(&self.state, cx, p) {
            Some(status) => status,
            None => sync_diff_list(&self.state, cx, p),
        };

        let footer = sync_footer(cx, can_start, selected_count, selection_state);

        crate::widgets::dialog_body(
            "sftp-sync-modal",
            rust_i18n::t!("sftp.sync.title").to_string(),
            Some(
                rust_i18n::t!(
                    "sftp.sync.subtitle",
                    local = local_root.as_str(),
                    remote = remote_root
                )
                .to_string(),
            ),
            content,
            Some(footer),
        )
    }
}

/// 扫描中/错误/无差异三种状态；返回 `None` 表示应渲染差异列表。
fn sync_status_panel(
    modal: &SftpSyncModalState,
    cx: &mut Context<SftpSyncModalView>,
    p: &Palette,
) -> Option<AnyElement> {
    let tab_id = modal.tab_id;
    let skipped = modal.skipped;
    let entries_empty = modal.entries.is_empty();
    if modal.scanning {
        Some(
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
                .into_any_element(),
        )
    } else if let Some(err) = modal.error.clone() {
        Some(
            v_flex()
                .h(px(360.0))
                .items_center()
                .justify_center()
                .gap_3()
                .child(div().text_sm().text_color(p.danger).child(err))
                .child(
                    Button::new("retry-sftp-sync-scan")
                        .ghost()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                        .label(rust_i18n::t!("sftp.sync.rescan").to_string())
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SftpSyncModalEvent::Rescan(tab_id));
                        })),
                )
                .into_any_element(),
        )
    } else if entries_empty {
        Some(
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
                .into_any_element(),
        )
    } else {
        None
    }
}

/// 差异列表：表头、虚拟列表与汇总行。
struct SyncDiffCounts {
    diff: usize,
    upload: usize,
    download: usize,
    conflict: usize,
    skipped: u64,
}

fn sync_diff_list(
    modal: &SftpSyncModalState,
    cx: &mut Context<SftpSyncModalView>,
    p: &Palette,
) -> AnyElement {
    let counts = count_sync_entries(modal);
    let list = render_sync_virtual_list(modal, cx, p);
    v_flex()
        .gap_2()
        .child(render_sync_table_header(p))
        .child(
            v_flex()
                .h(px(330.0))
                .overflow_hidden()
                .border_1()
                .border_color(p.border)
                .rounded(p.radius.md)
                .child(list),
        )
        .child(render_sync_summary(&counts, p))
        .into_any_element()
}

fn count_sync_entries(modal: &SftpSyncModalState) -> SyncDiffCounts {
    let upload = modal
        .entries
        .iter()
        .zip(modal.selected.iter())
        .filter(|(entry, checked)| {
            **checked
                && entry.status.suggested_direction()
                    == Some(transom_core::TransferDirection::Upload)
        })
        .count();
    let download = modal
        .entries
        .iter()
        .zip(modal.selected.iter())
        .filter(|(entry, checked)| {
            **checked
                && entry.status.suggested_direction()
                    == Some(transom_core::TransferDirection::Download)
        })
        .count();
    SyncDiffCounts {
        diff: modal.entries.len(),
        upload,
        download,
        conflict: modal
            .entries
            .iter()
            .filter(|entry| entry.status == DirDiffStatus::TypeMismatch)
            .count(),
        skipped: modal.skipped,
    }
}

fn render_sync_virtual_list(
    modal: &SftpSyncModalState,
    cx: &mut Context<SftpSyncModalView>,
    p: &Palette,
) -> AnyElement {
    let row_sizes = Rc::new(vec![size(px(0.0), px(32.0)); modal.entries.len()]);
    let entries = Rc::new(modal.entries.clone());
    let selected = Rc::new(modal.selected.clone());
    let palette = p.clone();
    v_virtual_list(
        cx.entity(),
        format!("sftp-sync-virtual-list-{}", modal.tab_id),
        row_sizes,
        move |_this, range, _window, cx| {
            range
                .filter_map(|index| {
                    let entry = entries.get(index)?;
                    let checked = selected.get(index).copied().unwrap_or(false);
                    Some(render_sync_row(index, entry, checked, &palette, cx))
                })
                .collect()
        },
    )
    .track_scroll(&modal.scroll_handle)
    .into_any_element()
}

fn render_sync_row(
    index: usize,
    entry: &DirDiffEntry,
    checked: bool,
    p: &Palette,
    cx: &mut Context<SftpSyncModalView>,
) -> AnyElement {
    let selectable = entry.status != DirDiffStatus::TypeMismatch;
    let (status_label, status_color) = status_presentation(entry.status, p);
    let mut row = h_flex()
        .id(ElementId::Name(format!("sftp-sync-row-{index}").into()))
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .text_xs()
        .border_b_1()
        .border_color(p.sidebar_border);
    if selectable {
        row = row.child(
            Checkbox::new(format!("sftp-sync-check-{index}"))
                .checked(checked)
                .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                    this.toggle_entry(index, *checked, cx);
                })),
        );
    } else {
        row = row.child(div().w(px(16.0)));
    }
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
            .text_color(p.muted_foreground)
            .child(side_text(entry.local.as_ref())),
    )
    .child(
        div()
            .w(px(132.0))
            .text_color(p.muted_foreground)
            .child(side_text(entry.remote.as_ref())),
    )
    .into_any_element()
}

fn render_sync_table_header(p: &Palette) -> AnyElement {
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
        )
        .into_any_element()
}

fn render_sync_summary(counts: &SyncDiffCounts, p: &Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(
            rust_i18n::t!(
                "sftp.sync.summary",
                diff = counts.diff,
                upload = counts.upload,
                download = counts.download,
                conflict = counts.conflict,
                skipped = counts.skipped
            )
            .to_string(),
        )
        .into_any_element()
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
