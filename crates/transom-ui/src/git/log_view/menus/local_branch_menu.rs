//! 本地分支右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 本地分支项的右键菜单分组口径；
//! - 动作直接落在所属 `Entity<GitWorkspace>` 上，跨窗口动作经实体事件上报，彻底解耦外壳；
//! - 参数收敛为专用目标结构体，符合复杂度控制与参数对象规范。

use crate::git::GitWorkspace;
use crate::widgets::menu_item;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::LogScope;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 本地分支菜单目标数据。
pub(crate) struct LocalBranchMenuTarget {
    pub(crate) branch: String,
    pub(crate) current_branch: Option<String>,
    pub(crate) upstream: Option<String>,
    pub(crate) is_current: bool,
}

/// 本地分支右键上下文菜单（对齐 IDEA 本地分支菜单）。
pub(crate) fn render_local_branch_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    target: LocalBranchMenuTarget,
) -> PopupMenu {
    let LocalBranchMenuTarget {
        branch,
        current_branch,
        upstream,
        is_current,
    } = target;

    let ws_new_branch = workspace.clone();
    let ws_worktree = workspace.clone();
    let ws_pull = workspace.clone();
    let ws_push = workspace.clone();
    let ws_rename = workspace.clone();
    let ws_delete = workspace.clone();
    let ws_diff = workspace.clone();
    let ws_upstream = workspace;

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
            move |_, _, cx| {
                let start = name_new_branch.clone();
                ws_new_branch.update(cx, |ws, cx| ws.create_branch_from(Some(start), cx));
            },
        ))
        // 2. 显示与工作树的差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let target = name_diff.clone();
                ws_diff.update(cx, |ws, cx| ws.set_log_scope(LogScope::Ref(target), cx));
            },
        ))
        // 3. 从 '{branch}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_worktree_from_ref", branch = branch.as_str()).to_string(),
            AppIcon::Ui(UiIcon::FOLDER),
            move |_, _, cx| {
                let start = name_worktree.clone();
                ws_worktree.update(cx, |ws, cx| ws.open_worktree_modal(start, cx));
            },
        ))
        .item(PopupMenuItem::separator())
        // 4. 更新
        .item(menu_item(
            rust_i18n::t!("git.menu.update").to_string(),
            AppIcon::Ui(UiIcon::REFRESH),
            move |_, _, cx| {
                ws_pull.update(cx, |ws, cx| ws.pull(false, cx));
            },
        ))
        // 5. 推送...
        .item(menu_item(
            rust_i18n::t!("git.menu.push").to_string(),
            AppIcon::Ui(UiIcon::UPLOAD),
            move |_, _, cx| {
                let target = name_push.clone();
                ws_push.update(cx, |ws, cx| ws.push_branch(Some(target), cx));
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
                ws_upstream.update(cx, |ws, cx| ws.set_log_scope(LogScope::Ref(target), cx));
            },
        ))
        .item(PopupMenuItem::separator())
        // 7. 重命名...
        .item(menu_item(
            rust_i18n::t!("git.menu.rename_branch").to_string(),
            AppIcon::Ui(UiIcon::EDIT),
            move |_, _, cx| {
                let old = name_rename.clone();
                ws_rename.update(cx, |ws, cx| ws.rename_branch(old, cx));
            },
        ))
        // 8. 删除（仅非当前检出分支允许删除）
        .when(!is_current, |m| {
            m.item(menu_item(
                rust_i18n::t!("git.menu.delete_ref").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
                move |_, _, cx| {
                    let name = name_delete.clone();
                    ws_delete.update(cx, |ws, cx| ws.delete_ref(&name, cx));
                },
            ))
        })
}
