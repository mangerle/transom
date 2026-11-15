//! 提交详情变更文件树节点的右键上下文菜单（全方位对齐 IntelliJ IDEA 变更树菜单）。
//!
//! 设计原理（Why）：
//! - 遵循 IDEA 变更树项的右键菜单分组口径；
//! - 第一组为核心比较与差异浏览（显示差异、新标签页差异、与本地比较）；
//! - 第二组为源码跳转（编辑源、打开仓库版本）；
//! - 第三组为精细化更改操作（还原、优选、提取到单独提交、创建补丁等）；
//! - 第四组为该文件的历史回溯与智能体协同（迄今历史、添加到智能体上下文）。

use crate::git::GitWorkspace;
use crate::widgets::{MenuItem, menu_item};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 变更文件树节点的右键上下文菜单。
pub(crate) fn render_file_tree_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    full_path: String,
) -> PopupMenu {
    let diff_ws = workspace.clone();
    let modal_ws = workspace.clone();
    let compare_ws = workspace.clone();
    let before_ws = workspace.clone();
    let edit_ws = workspace.clone();
    let repo_ws = workspace.clone();
    let revert_ws = workspace.clone();
    let delete_ws = workspace.clone();
    let history_ws = workspace.clone();
    let parent_ws = workspace.clone();
    let patch_ws = workspace.clone();
    let ai_ws = workspace;

    let path_diff = full_path.clone();
    let path_new_tab = full_path.clone();
    let path_compare = full_path.clone();
    let path_before = full_path.clone();
    let path_edit = full_path.clone();
    let path_repo = full_path.clone();
    let path_revert = full_path.clone();
    let path_delete = full_path.clone();
    let path_history = full_path.clone();
    let path_parent = full_path.clone();
    let path_patch = full_path.clone();
    let path_ai = full_path;

    menu
        // 1. 显示差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let path = path_diff.clone();
                diff_ws.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
            },
        ))
        // 2. 在新标签页中显示差异
        .item(menu_item(
            rust_i18n::t!("git.menu.show_diff_in_new_tab").to_string(),
            AppIcon::Ui(UiIcon::EXTERNAL_LINK),
            move |_, _, cx| {
                let path = path_new_tab.clone();
                modal_ws.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
            },
        ))
        // 3. 与本地比较
        .item(menu_item(
            rust_i18n::t!("git.menu.compare_with_local").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let path = path_compare.clone();
                compare_ws.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
            },
        ))
        // 4. 将之前版本与本地版本进行比较
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.compare_before_with_local").to_string(),
                AppIcon::Ui(UiIcon::DIFF),
            )
            .render(move |_, _, cx| {
                let path = path_before.clone();
                before_ws.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
            }),
        )
        .item(PopupMenuItem::separator())
        // 5. 编辑源
        .item(menu_item(
            rust_i18n::t!("git.menu.edit_source").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            move |_, _, cx| {
                let path = path_edit.clone();
                edit_ws.update(cx, |ws, cx| {
                    let full_file_path = ws.state.project.path.join(&path);
                    ws.copy_to_clipboard(&full_file_path.to_string_lossy(), cx);
                });
            },
        ))
        // 6. 打开仓库版本
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.open_repo_version").to_string(),
                AppIcon::Ui(UiIcon::DIFF),
            )
            .render(move |_, _, cx| {
                let path = path_repo.clone();
                repo_ws.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
            }),
        )
        .item(PopupMenuItem::separator())
        // 7. 还原所选更改
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.revert_selected_changes").to_string(),
                AppIcon::Ui(UiIcon::UNDO),
            )
            .render(move |_, _, cx| {
                let path = path_revert.clone();
                revert_ws.update(cx, |ws, cx| ws.rollback_single_path(&path, cx));
            }),
        )
        // 8. 优选所选更改
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.cherry_pick_selected_changes").to_string(),
                AppIcon::Ui(UiIcon::COMMIT),
            )
            .disabled(true)
            .render(|_, _, _| {}),
        )
        // 9. 将所选更改提取到单独的提交...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.extract_to_commit").to_string(),
                AppIcon::Ui(UiIcon::COMMIT),
            )
            .disabled(true)
            .render(|_, _, _| {}),
        )
        // 10. 删除所选更改
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.delete_selected_changes").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
            )
            .render(move |_, _, cx| {
                let path = path_delete.clone();
                delete_ws.update(cx, |ws, cx| ws.rollback_single_path(&path, cx));
            }),
        )
        // 11. 创建补丁...
        .item(menu_item(
            rust_i18n::t!("git.menu.create_patch").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            move |_, _, cx| {
                let path = path_patch.clone();
                patch_ws.update(cx, |ws, cx| ws.create_patch(&path, cx));
            },
        ))
        // 12. 从修订中获取
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.get_from_revision").to_string(),
                AppIcon::Ui(UiIcon::DOWNLOAD),
            )
            .disabled(true)
            .render(|_, _, _| {}),
        )
        .item(PopupMenuItem::separator())
        // 13. 迄今为止的历史记录
        .item(menu_item(
            rust_i18n::t!("git.menu.history_up_to_here").to_string(),
            AppIcon::Ui(UiIcon::HISTORY),
            move |_, _, cx| {
                let path = path_history.clone();
                history_ws.update(cx, |ws, cx| ws.set_log_path(Some(path), cx));
            },
        ))
        // 14. 显示对父项的更改
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.show_parent_changes").to_string(),
                AppIcon::Ui(UiIcon::DIFF),
            )
            .render(move |_, _, cx| {
                let path = path_parent.clone();
                parent_ws.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
            }),
        )
        // 15. 添加到智能体上下文
        .item(menu_item(
            rust_i18n::t!("git.menu.add_to_agent_context").to_string(),
            AppIcon::Ui(UiIcon::AI_INTENTION),
            move |_, _, cx| {
                let path = path_ai.clone();
                ai_ws.update(cx, |ws, cx| ws.copy_to_clipboard(&format!("@{path}"), cx));
            },
        ))
}
