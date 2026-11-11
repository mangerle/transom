//! 本地分支右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 本地分支项的右键菜单分组口径；
//! - 贯穿确定性 `GitProject` 与 `TabId`，彻底根除跨标签页盲查项目失败导致的点击静默失效；
//! - 参数收敛为专用目标结构体，符合复杂度控制与参数对象规范。

use crate::app::AppShell;
use crate::widgets::menu_item;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitProject, LogScope, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 本地分支菜单目标数据。
pub(crate) struct LocalBranchMenuTarget {
    pub(crate) project: GitProject,
    pub(crate) tab_id: TabId,
    pub(crate) branch: String,
    pub(crate) current_branch: Option<String>,
    pub(crate) upstream: Option<String>,
    pub(crate) is_current: bool,
}

/// 本地分支右键上下文菜单（对齐 IDEA 本地分支菜单）。
pub(crate) fn render_local_branch_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    target: LocalBranchMenuTarget,
) -> PopupMenu {
    let LocalBranchMenuTarget {
        project,
        tab_id,
        branch,
        current_branch,
        upstream,
        is_current,
    } = target;

    let app_new_branch = app.clone();
    let app_worktree = app.clone();
    let app_pull = app.clone();
    let app_push = app.clone();
    let app_rename = app.clone();
    let app_delete = app.clone();
    let app_diff = app.clone();
    let app_upstream = app;

    let project_new_branch = project.clone();
    let project_worktree = project.clone();
    let project_pull = project.clone();
    let project_push = project.clone();
    let project_rename = project.clone();
    let project_delete = project;

    let name_new_branch = branch.clone();
    let name_worktree = branch.clone();
    let name_rename = branch.clone();
    let name_delete = branch.clone();
    let name_diff = branch.clone();
    let name_push = branch.clone();

    let upstream_display = upstream.unwrap_or_else(|| {
        format!(
            "origin/{}",
            current_branch.as_deref().unwrap_or(branch.as_str())
        )
    });
    let upstream_click = upstream_display.clone();

    menu
        // 1. 从 '{branch}' 新建分支...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_branch_from_ref", branch = branch.as_str()).to_string(),
            AppIcon::Ui(UiIcon::VCS),
            move |_, window, cx| {
                let start = name_new_branch.clone();
                let project = project_new_branch.clone();
                app_new_branch.update(cx, |this, cx| {
                    this.git_create_project_branch_from(&project, &start, window, cx);
                });
            },
        ))
        // 2. 显示与工作树的差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let target = name_diff.clone();
                app_diff.update(cx, |this, cx| {
                    this.git_set_log_scope(tab_id, LogScope::Ref(target), cx);
                });
            },
        ))
        // 3. 从 '{branch}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_worktree_from_ref", branch = branch.as_str()).to_string(),
            AppIcon::Ui(UiIcon::FOLDER),
            move |_, window, cx| {
                let start = name_worktree.clone();
                let project = project_worktree.clone();
                app_worktree.update(cx, |this, cx| {
                    this.open_git_worktree_modal(project, &start, window, cx);
                });
            },
        ))
        .item(PopupMenuItem::separator())
        // 4. 更新
        .item(menu_item(
            rust_i18n::t!("git.menu.update").to_string(),
            AppIcon::Ui(UiIcon::REFRESH),
            move |_, _, cx| {
                let project = project_pull.clone();
                app_pull.update(cx, |this, cx| {
                    this.git_pull_project(&project, false, cx);
                });
            },
        ))
        // 5. 推送...
        .item(menu_item(
            rust_i18n::t!("git.menu.push").to_string(),
            AppIcon::Ui(UiIcon::UPLOAD),
            move |_, _, cx| {
                let target = name_push.clone();
                let project = project_push.clone();
                app_push.update(cx, |this, cx| {
                    this.git_push_project(&project, Some(target), cx);
                });
            },
        ))
        // 6. 跟踪的分支 '{upstream}'
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.tracked_branch",
                upstream = upstream_display.as_str()
            )
            .to_string(),
            AppIcon::Ui(UiIcon::VCS),
            move |_, _, cx| {
                let target = upstream_click.clone();
                app_upstream.update(cx, |this, cx| {
                    this.git_set_log_scope(tab_id, LogScope::Ref(target), cx);
                });
            },
        ))
        .item(PopupMenuItem::separator())
        // 7. 重命名...
        .item(menu_item(
            rust_i18n::t!("git.menu.rename_branch").to_string(),
            AppIcon::Ui(UiIcon::EDIT),
            move |_, window, cx| {
                let old = name_rename.clone();
                let project = project_rename.clone();
                app_rename.update(cx, |this, cx| {
                    this.open_git_name_modal(
                        &project,
                        crate::git::GitNamePurpose::RenameBranch { old: old.clone() },
                        old,
                        window,
                        cx,
                    );
                });
            },
        ))
        // 8. 删除（仅非当前检出分支允许删除）
        .when(!is_current, |m| {
            m.item(menu_item(
                rust_i18n::t!("git.menu.delete_ref").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
                move |_, _, cx| {
                    let name = name_delete.clone();
                    let project = project_delete.clone();
                    app_delete.update(cx, |this, cx| {
                        this.git_delete_project_ref(&project, &name, cx);
                    });
                },
            ))
        })
}
