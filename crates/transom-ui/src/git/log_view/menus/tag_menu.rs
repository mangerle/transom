//! 标签右键上下文菜单（全方位对齐 IntelliJ IDEA 引用上下文菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 标签项的右键菜单分组口径；
//! - 动作直接落在所属 `Entity<GitWorkspace>` 上，跨窗口动作经实体事件上报，彻底解耦外壳；
//! - 参数收敛为专用目标结构体，符合复杂度控制与参数对象规范。

use crate::git::GitWorkspace;
use crate::git::actions::GitHistoryAction;
use crate::widgets::menu_item;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use transom_core::LogScope;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 标签菜单目标数据。
pub(crate) struct TagMenuTarget {
    pub(crate) tag: String,
    pub(crate) current_branch: Option<String>,
}

/// 标签右键上下文菜单（对齐 IDEA 标签菜单）。
pub(crate) fn render_tag_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    target: TagMenuTarget,
) -> PopupMenu {
    let TagMenuTarget {
        tag,
        current_branch,
    } = target;

    let cur = current_branch.unwrap_or_else(|| "main".to_string());
    let ws_checkout = workspace.clone();
    let ws_new_branch = workspace.clone();
    let ws_merge = workspace.clone();
    let ws_worktree = workspace.clone();
    let ws_push = workspace.clone();
    let ws_delete = workspace.clone();
    let ws_diff = workspace;

    let name_checkout = tag.clone();
    let name_new_branch = tag.clone();
    let name_merge = tag.clone();
    let name_worktree = tag.clone();
    let name_push = tag.clone();
    let name_delete = tag.clone();
    let name_diff = tag.clone();

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
        // 2. 从 '{tag}' 新建分支...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_branch_from_ref", branch = tag.as_str()).to_string(),
            AppIcon::Ui(UiIcon::VCS),
            move |_, _, cx| {
                let start = name_new_branch.clone();
                ws_new_branch.update(cx, |ws, cx| ws.create_branch_from(Some(start), cx));
            },
        ))
        // 3. 显示与工作树的差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_with_worktree").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let target = name_diff.clone();
                ws_diff.update(cx, |ws, cx| ws.set_log_scope(LogScope::Ref(target), cx));
            },
        ))
        // 4. 将 '{tag}' 合并到 '{current}' 中
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.merge_tag_into",
                tag = tag.as_str(),
                branch = cur.as_str()
            )
            .to_string(),
            AppIcon::Ui(UiIcon::MERGE),
            move |_, _, cx| {
                let reference = name_merge.clone();
                ws_merge.update(cx, |ws, cx| {
                    ws.history_action(GitHistoryAction::Merge(reference), cx);
                });
            },
        ))
        // 5. 从 '{tag}' 新建工作树...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_worktree_from_ref", branch = tag.as_str()).to_string(),
            AppIcon::Ui(UiIcon::FOLDER),
            move |_, _, cx| {
                let branch = name_worktree.clone();
                ws_worktree.update(cx, |ws, cx| ws.open_worktree_modal(branch, cx));
            },
        ))
        // 6. 推送到 origin
        .item(menu_item(
            rust_i18n::t!("git.menu.push_to_remote", remote = "origin").to_string(),
            AppIcon::Ui(UiIcon::UPLOAD),
            move |_, _, cx| {
                let name = name_push.clone();
                ws_push.update(cx, |ws, cx| {
                    ws.push_branch(Some(format!("refs/tags/{name}")), cx);
                });
            },
        ))
        // 7. 删除
        .item(menu_item(
            rust_i18n::t!("git.menu.delete_ref").to_string(),
            AppIcon::Ui(UiIcon::DELETE),
            move |_, _, cx| {
                let name = name_delete.clone();
                ws_delete.update(cx, |ws, cx| ws.delete_ref(&name, cx));
            },
        ))
}
