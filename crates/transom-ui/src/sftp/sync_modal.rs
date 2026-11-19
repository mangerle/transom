//! 目录对比与增量同步弹窗视图实体。

use super::{SftpSyncModalState, format_size, format_unix_time};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Sizable, v_virtual_list};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use std::rc::Rc;
use transom_core::{ConnectionId, DirComparison, DirDiffEntry, DirDiffStatus, TabId, TransferId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

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
fn sync_diff_list(
    modal: &SftpSyncModalState,
    cx: &mut Context<SftpSyncModalView>,
    p: &Palette,
) -> AnyElement {
    let tab_id = modal.tab_id;
    let entries = modal.entries.clone();
    let selected = modal.selected.clone();
    let scroll_handle = modal.scroll_handle.clone();
    let skipped = modal.skipped;
    let diff_count = entries.len();
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

    let row_sizes = Rc::new(vec![size(px(0.0), px(32.0)); entries.len()]);
    let entries_rc = Rc::new(entries);
    let selected_rc = Rc::new(selected);
    let list_palette = p.clone();
    let list = v_virtual_list(
        cx.entity(),
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
                                .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.toggle_entry(index, *checked, cx);
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
}

/// 组级全选状态；用三态枚举避免“全部选中”和“部分选中”两个布尔值
/// 出现同时为真或同时为假的非法组合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncSelectAllState {
    None,
    Partial,
    All,
}

/// 计算可同步条目的组级全选状态；类型冲突行不参与全选。
fn sync_select_all_state(entries: &[DirDiffEntry], selected: &[bool]) -> SyncSelectAllState {
    let mut selectable = 0usize;
    let mut checked = 0usize;
    for (entry, selected) in entries.iter().zip(selected.iter()) {
        if entry.status == DirDiffStatus::TypeMismatch {
            continue;
        }
        selectable = selectable.saturating_add(1);
        if *selected {
            checked = checked.saturating_add(1);
        }
    }
    if checked == 0 {
        SyncSelectAllState::None
    } else if checked == selectable {
        SyncSelectAllState::All
    } else {
        SyncSelectAllState::Partial
    }
}

/// 同步弹窗页脚：取消、组级全选与开始同步。
fn sync_footer(
    cx: &mut Context<SftpSyncModalView>,
    can_start: bool,
    selected_count: usize,
    selection_state: SyncSelectAllState,
) -> AnyElement {
    let select_all_label = rust_i18n::t!("sftp.sync.select_all").to_string();
    let footer = h_flex()
        .gap_2()
        .child(crate::widgets::dialog_cancel_button(
            "cancel-sftp-sync",
            rust_i18n::t!("common.cancel").to_string(),
            cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)),
        ))
        .when(can_start, |this| {
            this.child(
                Checkbox::new("select-all-sftp-sync")
                    .small()
                    .checked(selection_state == SyncSelectAllState::All)
                    .indeterminate(selection_state == SyncSelectAllState::Partial)
                    .label(select_all_label.clone())
                    .accessibility_label(select_all_label)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.select_all(*checked, cx);
                    })),
            )
            .child(
                Button::new("start-sftp-sync")
                    .primary()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::DIFF)).small())
                    .label(rust_i18n::t!("sftp.sync.start", count = selected_count).to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.submit(cx);
                    })),
            )
        });
    footer.into_any_element()
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

#[cfg(test)]
mod tests {
    use super::{SyncSelectAllState, sync_select_all_state};
    use transom_core::{DirDiffEntry, DirDiffStatus, DirEntryMeta};

    fn entry(status: DirDiffStatus) -> DirDiffEntry {
        DirDiffEntry {
            relative_path: "file".to_string(),
            local: Some(DirEntryMeta {
                is_dir: false,
                size: 1,
                mtime: None,
            }),
            remote: Some(DirEntryMeta {
                is_dir: false,
                size: 1,
                mtime: None,
            }),
            status,
        }
    }

    #[test]
    fn select_all_state_ignores_type_conflicts() {
        let entries = [
            entry(DirDiffStatus::LocalOnly),
            entry(DirDiffStatus::RemoteOnly),
            entry(DirDiffStatus::TypeMismatch),
        ];
        assert_eq!(
            sync_select_all_state(&entries, &[false, false, false]),
            SyncSelectAllState::None
        );
        assert_eq!(
            sync_select_all_state(&entries, &[true, false, false]),
            SyncSelectAllState::Partial
        );
        assert_eq!(
            sync_select_all_state(&entries, &[true, true, false]),
            SyncSelectAllState::All
        );
    }

    #[test]
    fn select_all_state_has_no_partial_or_all_without_selectable_entries() {
        let entries = [entry(DirDiffStatus::TypeMismatch)];
        assert_eq!(
            sync_select_all_state(&entries, &[true]),
            SyncSelectAllState::None
        );
        assert_eq!(sync_select_all_state(&[], &[]), SyncSelectAllState::None);
    }
}
