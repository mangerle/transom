//! 远程分支右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 远程分支项的右键菜单分组口径；
//! - 动作直接落在所属 `Entity<GitWorkspace>` 上，跨窗口动作经实体事件上报，彻底解耦外壳；
//! - 参数收敛为专用目标结构体，符合复杂度控制与参数对象规范。

use crate::git::GitWorkspace;
use crate::git::actions::GitHistoryAction;
use crate::widgets::menu_item;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::LogScope;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 远程分支菜单目标数据。
pub(crate) struct RemoteBranchMenuTarget {
    pub(crate) remote_branch: String,
    pub(crate) current_branch: Option<String>,
}

/// 远程分支右键上下文菜单（对齐 IDEA 远程分支菜单）。
pub(crate) fn render_remote_branch_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    target: RemoteBranchMenuTarget,
) -> PopupMenu {
    let RemoteBranchMenuTarget {
        remote_branch,
        current_branch,
    } = target;

    let cur = current_branch.unwrap_or_else(|| "main".to_string());
    let ws_checkout = workspace.clone();
    let ws_new_branch = workspace.clone();
    let ws_rebase_into = workspace.clone();
    let ws_merge_into = workspace.clone();
    let ws_pull_rebase = workspace.clone();
    let ws_pull_merge = workspace.clone();
    let ws_worktree = workspace.clone();
    let ws_delete = workspace.clone();
    let ws_diff = workspace.clone();
    let ws_compare = workspace;

    let name_checkout = remote_branch.clone();
    let name_new_branch = remote_branch.clone();
    let name_rebase = remote_branch.clone();
    let name_merge = remote_branch.clone();
    let name_pull_rebase = remote_branch.clone();
    let name_pull_merge = remote_branch.clone();
    let name_worktree = remote_branch.clone();
    let name_delete = remote_branch.clone();
    let name_diff = remote_branch.clone();
    let name_compare = remote_branch.clone();

    menu
        // 1. 签出
        .item(menu_item(
            rust_i18n::t!("git.menu.checkout").to_string(),
            AppIcon::Ui(UiIcon::RIGHT),
            move |_, _, cx| {
                let name = name_checkout.clone();
                ws_checkout.update(cx, |ws, cx| ws.checkout_ref(&name, cx));
            },
        ))
        // 2. 从 '{remote_branch}' 新建分支...
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.new_branch_from_ref",
                branch = remote_branch.as_str()
            )
            .to_string(),
            AppIcon::Ui(UiIcon::VCS),
            move |_, _, cx| {
                let start = name_new_branch.clone();
                ws_new_branch.update(cx, |ws, cx| ws.create_branch_from(Some(start), cx));
            },
        ))
        // 3. 与 '{current}' 比较
        .item(menu_item(
            rust_i18n::t!("git.menu.compare_with_branch", current = cur.as_str()).to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let target = name_compare.clone();
                ws_compare.update(cx, |ws, cx| ws.set_log_scope(LogScope::Ref(target), cx));
            },
        ))
        // 4. 显示与工作树的差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let target = name_diff.clone();
                ws_diff.update(cx, |ws, cx| ws.set_log_scope(LogScope::Ref(target), cx));
            },
        ))
        .item(PopupMenuItem::separator())
        // 5. 将 '{current}' 变基到 '{remote_branch}'
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.rebase_current_onto_ref",
                current = cur.as_str(),
                branch = remote_branch.as_str()
            )
            .to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let upstream = name_rebase.clone();
                ws_rebase_into.update(cx, |ws, cx| {
                    ws.history_action(GitHistoryAction::Rebase(upstream), cx);
                });
            },
        ))
        // 6. 将 '{remote_branch}' 合并到 '{current}' 中
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.merge_ref_into_current",
                branch = remote_branch.as_str(),
                current = cur.as_str()
            )
            .to_string(),
            AppIcon::Ui(UiIcon::MERGE),
            move |_, _, cx| {
                let reference = name_merge.clone();
                ws_merge_into.update(cx, |ws, cx| {
                    ws.history_action(GitHistoryAction::Merge(reference), cx);
                });
            },
        ))
        // 7. 从 '{remote_branch}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.new_worktree_from_ref",
                branch = remote_branch.as_str()
            )
            .to_string(),
            AppIcon::Ui(UiIcon::FOLDER),
            move |_, _, cx| {
                let branch = name_worktree.clone();
                ws_worktree.update(cx, |ws, cx| ws.open_worktree_modal(branch, cx));
            },
        ))
        // 8. 使用“变基”拉入 '{current}'
        .item(menu_item(
            rust_i18n::t!("git.menu.pull_with_rebase", current = cur.as_str()).to_string(),
            AppIcon::Ui(UiIcon::REFRESH),
            move |_, _, cx| {
                let upstream = name_pull_rebase.clone();
                ws_pull_rebase.update(cx, |ws, cx| {
                    ws.history_action(GitHistoryAction::Rebase(upstream), cx);
                });
            },
        ))
        // 9. 使用“合并”拉入 '{current}'
        .item(menu_item(
            rust_i18n::t!("git.menu.pull_with_merge", current = cur.as_str()).to_string(),
            AppIcon::Ui(UiIcon::REFRESH),
            move |_, _, cx| {
                let reference = name_pull_merge.clone();
                ws_pull_merge.update(cx, |ws, cx| {
                    ws.history_action(GitHistoryAction::Merge(reference), cx);
                });
            },
        ))
        .item(PopupMenuItem::separator())
        // 10. 删除
        .item(menu_item(
            rust_i18n::t!("git.menu.delete_ref").to_string(),
            AppIcon::Ui(UiIcon::DELETE),
            move |_, _, cx| {
                let name = name_delete.clone();
                ws_delete.update(cx, |ws, cx| ws.delete_ref(&name, cx));
            },
        ))
}
