//! Git 差异视图行槽位（Gutter Marker）变更块操作：暂存块 / 取消暂存块 / 还原块。
//!
//! 设计原理（Why）：
//! - 利用 `gpui_kit::Diff::gutter_markers` 在每个变更块起始行渲染紧凑动作按钮，
//!   无需自绘叠层即可在并排/统一两种模式下精确跟随虚拟滚动与折叠展开；
//! - 工作区（Unstaged）提供弹出菜单区分「暂存 / 还原」，避免单次误触直接覆盖本地代码；
//!   已暂存（Staged）提供一键撤销暂存，保持高频操作最短路径。

use std::collections::HashMap;
use std::rc::Rc;

use crate::app::AppShell;
use crate::git::types::{GitDiffState, HunkActionCapability};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::{Diff, DiffGutterMarker, DiffLinePosition};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::{AnyElement, ElementId, Entity, IntoElement, ParentElement, SharedString, div};
use transom_core::{GitAction, RepoStatus, TabId};
use transom_ui_icons::{AppIconElement, UiIcon};

/// 当当前差异对比支持块级写操作时，向 `Diff` 控件注入槽位按钮与补丁应用回调。
pub(super) fn attach_hunk_actions(
    diff_widget: Diff,
    diff: &GitDiffState,
    status: Option<&RepoStatus>,
    app: Entity<AppShell>,
    tab_id: TabId,
) -> Diff {
    let capability = diff.hunk_capability(status);
    if capability == HunkActionCapability::None {
        return diff_widget;
    }

    let mut markers = Vec::with_capacity(diff.hunks.len());
    let mut patch_map: HashMap<DiffLinePosition, String> = HashMap::with_capacity(diff.hunks.len());
    let label: SharedString = match capability {
        HunkActionCapability::Unstaged => rust_i18n::t!("git.diff.hunk_actions").into(),
        HunkActionCapability::Staged => rust_i18n::t!("git.diff.unstage_hunk").into(),
        HunkActionCapability::None => return diff_widget,
    };

    for (idx, hunk) in diff.hunks.iter().enumerate() {
        let pos = hunk.gutter_position();
        patch_map.insert(pos.clone(), hunk.patch.clone());
        let id = ElementId::Name(format!("git-diff-hunk-{}-{idx}", diff.request_id).into());
        markers.push(DiffGutterMarker::new(id, pos).with_label(label.clone()));
    }
    let patch_map = Rc::new(patch_map);

    diff_widget.gutter_markers(markers).render_gutter_marker(
        move |marker, _window, _cx| -> AnyElement {
            let Some(patch) = patch_map.get(marker.position()).cloned() else {
                return div().into_any_element();
            };
            match capability {
                HunkActionCapability::Unstaged => {
                    render_unstaged_hunk_button(marker.id().clone(), patch, app.clone(), tab_id)
                }
                HunkActionCapability::Staged => {
                    render_staged_hunk_button(marker.id().clone(), patch, app.clone(), tab_id)
                }
                HunkActionCapability::None => div().into_any_element(),
            }
        },
    )
}

fn render_unstaged_hunk_button(
    id: ElementId,
    patch: String,
    app: Entity<AppShell>,
    tab_id: TabId,
) -> AnyElement {
    let tooltip = rust_i18n::t!("git.diff.hunk_actions").to_string();
    let stage_patch = patch.clone();
    let revert_patch = patch;
    let stage_app = app.clone();
    let revert_app = app;
    Button::new(id)
        .ghost()
        .xsmall()
        .child(AppIconElement::new(UiIcon::MORE_HORIZONTAL))
        .accessibility_label(tooltip.clone())
        .tooltip(tooltip)
        .dropdown_menu(move |menu, _window, _cx| {
            let s_patch = stage_patch.clone();
            let r_patch = revert_patch.clone();
            let s_app = stage_app.clone();
            let r_app = revert_app.clone();
            menu.item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.stage_hunk").to_string()).on_click(
                    move |_, _, cx| {
                        let patch = s_patch.clone();
                        s_app.update(cx, |this, cx| {
                            this.git_apply_hunk_patch(
                                tab_id,
                                GitAction::StageHunk,
                                patch,
                                true,
                                false,
                                cx,
                            );
                        });
                    },
                ),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.revert_hunk").to_string()).on_click(
                    move |_, _, cx| {
                        let patch = r_patch.clone();
                        r_app.update(cx, |this, cx| {
                            this.git_apply_hunk_patch(
                                tab_id,
                                GitAction::RevertHunk,
                                patch,
                                false,
                                true,
                                cx,
                            );
                        });
                    },
                ),
            )
        })
        .into_any_element()
}

fn render_staged_hunk_button(
    id: ElementId,
    patch: String,
    app: Entity<AppShell>,
    tab_id: TabId,
) -> AnyElement {
    let tooltip = rust_i18n::t!("git.diff.unstage_hunk").to_string();
    Button::new(id)
        .ghost()
        .xsmall()
        .child(AppIconElement::new(UiIcon::UNDO))
        .accessibility_label(tooltip.clone())
        .tooltip(tooltip)
        .on_click(move |_, _, cx| {
            let patch = patch.clone();
            app.update(cx, |this, cx| {
                this.git_apply_hunk_patch(tab_id, GitAction::UnstageHunk, patch, true, true, cx);
            });
        })
        .into_any_element()
}
