//! Git 日志引用栏左侧动作条。
//!
//! 设计原理（Why）：动作条按 IDEA / openidea 的分支面板组织，顶部只保留面板控制与
//! 引用写操作，底部集中放置刷新、设置和树展开控制。动作作用于当前日志范围指向的
//! 引用，而不是另建一份容易与日志范围脱节的选中状态。

use super::current_head_name;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_core::{BranchEntry, LogScope, RefKind, TabId, TagEntry};

type State = crate::git::GitWorkspaceState;

/// 当前日志范围对应的可操作引用。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefToolbarTarget {
    name: String,
    kind: RefKind,
    current: bool,
    updatable: bool,
    deletable: bool,
}

/// 渲染引用栏左侧动作条。
pub(crate) fn render_refs_action_bar(
    state: &State,
    app: Entity<AppShell>,
    tab_id: TabId,
    p: &Palette,
) -> AnyElement {
    let target = selected_target(state);
    let new_branch_start = target
        .as_ref()
        .map(|target| target.name.clone())
        .unwrap_or_else(|| "HEAD".to_string());
    let can_update = target.as_ref().is_some_and(|target| target.updatable);
    let can_delete = target.as_ref().is_some_and(|target| target.deletable);
    let delete_name = target.as_ref().map(|target| target.name.clone());
    let project = state.project.clone();
    let refresh_project = project.clone();
    let fetch_project = project.clone();
    let pull_project = project.clone();

    let hide_app = app.clone();
    let new_app = app.clone();
    let update_app = app.clone();
    let delete_app = app.clone();
    let fetch_app = app.clone();
    let refresh_app = app.clone();
    let settings_app = app.clone();
    let expand_app = app.clone();
    let collapse_app = app.clone();

    v_flex()
        .w(px(26.0))
        .flex_none()
        .h_full()
        .items_center()
        .py_1()
        .gap_0p5()
        .bg(p.muted)
        .border_color(p.divider(p.border))
        .border_r_1()
        .child(
            toolbar_button(
                "git-refs-collapse-btn",
                IconName::ChevronLeft,
                rust_i18n::t!("git.refs.hide").to_string(),
            )
            .on_click(move |_, _, cx| {
                hide_app.update(cx, |this, cx| {
                    this.git_toggle_refs_panel(tab_id, cx);
                });
            }),
        )
        .child(toolbar_separator(p))
        .child(
            toolbar_button(
                "git-refs-new-branch-btn",
                IconName::Plus,
                rust_i18n::t!("git.refs.new_branch").to_string(),
            )
            .on_click(move |_, window, cx| {
                let start = new_branch_start.clone();
                new_app.update(cx, |this, cx| {
                    this.git_create_branch_from(&start, window, cx);
                });
            }),
        )
        .child(
            toolbar_button(
                "git-refs-update-btn",
                IconName::ArrowDownToLine,
                rust_i18n::t!("git.refs.update_selected").to_string(),
            )
            .disabled(!can_update)
            .on_click(move |_, _, cx| {
                update_app.update(cx, |this, cx| {
                    this.git_remote_action(
                        &pull_project,
                        crate::git::actions::GitRemoteAction::Pull { rebase: false },
                        cx,
                    );
                });
            }),
        )
        .child(
            toolbar_button(
                "git-refs-delete-btn",
                IconName::Trash,
                rust_i18n::t!("git.refs.delete_selected").to_string(),
            )
            .disabled(!can_delete)
            .on_click(move |_, _, cx| {
                let Some(name) = delete_name.as_ref() else {
                    return;
                };
                delete_app.update(cx, |this, cx| {
                    this.git_delete_project_ref(&project, name, cx);
                });
            }),
        )
        .child(
            toolbar_button(
                "git-refs-fetch-btn",
                IconName::Download,
                rust_i18n::t!("git.toolbar.fetch").to_string(),
            )
            .on_click(move |_, _, cx| {
                fetch_app.update(cx, |this, cx| {
                    this.git_remote_action(
                        &fetch_project,
                        crate::git::actions::GitRemoteAction::Fetch,
                        cx,
                    );
                });
            }),
        )
        .child(
            toolbar_button(
                "git-refs-refresh-btn",
                IconName::RotateCw,
                rust_i18n::t!("git.menu.refresh").to_string(),
            )
            .on_click(move |_, _, cx| {
                refresh_app.update(cx, |this, cx| {
                    this.git_refresh(&refresh_project, cx);
                    this.git_load_log(tab_id, 0, cx);
                });
            }),
        )
        .child(toolbar_separator(p))
        .child(
            toolbar_button(
                "git-refs-settings-btn",
                IconName::Settings,
                rust_i18n::t!("git.refs.settings").to_string(),
            )
            .on_click(move |_, window, cx| {
                settings_app.update(cx, |this, cx| {
                    this.open_git_settings(window, cx);
                });
            }),
        )
        .child(
            toolbar_button(
                "git-refs-expand-all-btn",
                IconName::ChevronsUpDown,
                rust_i18n::t!("git.group.expand_all").to_string(),
            )
            .on_click(move |_, _, cx| {
                expand_app.update(cx, |this, cx| {
                    this.git_set_all_ref_groups_expanded(true, cx);
                });
            }),
        )
        .child(
            toolbar_button(
                "git-refs-collapse-all-btn",
                IconName::ChevronsDownUp,
                rust_i18n::t!("git.group.collapse_all").to_string(),
            )
            .on_click(move |_, _, cx| {
                collapse_app.update(cx, |this, cx| {
                    this.git_set_all_ref_groups_expanded(false, cx);
                });
            }),
        )
        .into_any_element()
}

/// 构造一个 IDEA 风格的图标动作按钮。
fn toolbar_button(id: &'static str, icon: IconName, tooltip: String) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .accessibility_label(tooltip.clone())
        .tooltip(tooltip)
}

/// 动作条内的轻量分组分隔线。
fn toolbar_separator(p: &Palette) -> AnyElement {
    div()
        .w(px(16.0))
        .h(px(1.0))
        .my_1()
        .bg(p.divider(p.border))
        .into_any_element()
}

/// 将日志范围解析为动作条可操作的目标。
fn selected_target(state: &State) -> Option<RefToolbarTarget> {
    let (name, explicit_kind) = match &state.query.scope {
        LogScope::CurrentBranch => (current_head_name(state)?, Some(RefKind::HeadAt)),
        LogScope::Ref(name) => (name.clone(), None),
        LogScope::AllRefs => return None,
    };
    target_from_refs(&name, explicit_kind, &state.branches, &state.tags)
}

/// 根据分支与标签列表判断引用类型及可用动作。
fn target_from_refs(
    name: &str,
    explicit_kind: Option<RefKind>,
    branches: &[BranchEntry],
    tags: &[TagEntry],
) -> Option<RefToolbarTarget> {
    if let Some(branch) = branches.iter().find(|branch| branch.name == name) {
        let kind = if branch.remote {
            RefKind::RemoteBranch
        } else if branch.current {
            RefKind::HeadAt
        } else {
            RefKind::Branch
        };
        return Some(RefToolbarTarget {
            name: branch.name.clone(),
            kind,
            current: branch.current,
            // 当前命令层只支持拉取当前检出分支；非当前分支更新留待独立命令落地。
            updatable: !branch.remote && branch.current,
            // 远端跟踪引用的删除需要 push --delete，不能复用本地 branch -d。
            deletable: !branch.remote && !branch.current,
        });
    }
    if tags.iter().any(|tag| tag.name == name) {
        return Some(RefToolbarTarget {
            name: name.to_string(),
            kind: RefKind::Tag,
            current: false,
            updatable: false,
            deletable: true,
        });
    }
    explicit_kind.map(|kind| RefToolbarTarget {
        name: name.to_string(),
        kind,
        current: matches!(kind, RefKind::HeadAt),
        updatable: matches!(kind, RefKind::HeadAt),
        deletable: false,
    })
}

#[cfg(test)]
mod tests {
    use super::target_from_refs;
    use transom_core::{BranchEntry, RefKind, TagEntry};

    fn branch(name: &str, remote: bool, current: bool) -> BranchEntry {
        BranchEntry {
            name: name.to_string(),
            remote,
            oid: String::new(),
            current,
            upstream: None,
            ahead: 0,
            behind: 0,
        }
    }

    fn tag(name: &str) -> TagEntry {
        TagEntry {
            name: name.to_string(),
            oid: String::new(),
            target: String::new(),
            message: None,
        }
    }

    /// 当前分支可更新，但不能通过动作条删除。
    #[test]
    fn current_branch_is_updatable_but_not_deletable() {
        let branches = vec![branch("main", false, true)];
        let target =
            target_from_refs("main", Some(RefKind::HeadAt), &branches, &[]).expect("目标存在");
        assert!(target.current);
        assert!(target.updatable);
        assert!(!target.deletable);
    }

    /// 普通本地分支可删除，但当前拉取命令不能更新非当前分支。
    #[test]
    fn other_local_branch_is_deletable_but_not_updatable() {
        let branches = vec![branch("feature", false, false)];
        let target = target_from_refs("feature", None, &branches, &[]).expect("目标存在");
        assert!(!target.current);
        assert!(!target.updatable);
        assert!(target.deletable);
    }

    /// 远端跟踪引用可浏览和作为新建分支起点，但不能误用本地删除命令。
    #[test]
    fn remote_branch_is_not_deletable_from_local_toolbar() {
        let branches = vec![branch("origin/main", true, false)];
        let target = target_from_refs("origin/main", None, &branches, &[]).expect("目标存在");
        assert_eq!(target.kind, RefKind::RemoteBranch);
        assert!(!target.updatable);
        assert!(!target.deletable);
    }

    /// 标签可以直接删除。
    #[test]
    fn tag_is_deletable() {
        let tags = vec![tag("v1.0.0")];
        let target = target_from_refs("v1.0.0", None, &[], &tags).expect("目标存在");
        assert_eq!(target.kind, RefKind::Tag);
        assert!(target.deletable);
    }
}
