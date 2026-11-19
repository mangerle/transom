//! SFTP 同步弹窗页脚与组级全选状态。

use super::SftpSyncModalView;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{DirDiffEntry, DirDiffStatus};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 组级全选状态；用三态枚举避免“全部选中”和“部分选中”两个布尔值
/// 出现同时为真或同时为假的非法组合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SyncSelectAllState {
    None,
    Partial,
    All,
}

/// 计算可同步条目的组级全选状态；类型冲突行不参与全选。
pub(super) fn sync_select_all_state(
    entries: &[DirDiffEntry],
    selected: &[bool],
) -> SyncSelectAllState {
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
pub(super) fn sync_footer(
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
