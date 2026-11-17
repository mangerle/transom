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
    let menu = append_diff_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.show_diff",
        UiIcon::DIFF,
    );
    let menu = append_diff_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.show_diff_in_new_tab",
        UiIcon::EXTERNAL_LINK,
    );
    let menu = append_diff_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.compare_with_local",
        UiIcon::DIFF,
    );
    let menu = append_diff_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.compare_before_with_local",
        UiIcon::DIFF,
    );
    let menu = menu.item(PopupMenuItem::separator());
    let menu = append_edit_source(menu, workspace.clone(), &full_path);
    let menu = append_diff_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.open_repo_version",
        UiIcon::DIFF,
    );
    let menu = menu.item(PopupMenuItem::separator());
    let menu = append_rollback_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.revert_selected_changes",
        UiIcon::UNDO,
    );
    let menu = append_disabled_item(
        menu,
        "git.menu.cherry_pick_selected_changes",
        UiIcon::COMMIT,
    );
    let menu = append_disabled_item(menu, "git.menu.extract_to_commit", UiIcon::COMMIT);
    let menu = append_rollback_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.delete_selected_changes",
        UiIcon::DELETE,
    );
    let menu = append_create_patch(menu, workspace.clone(), &full_path);
    let menu = append_disabled_item(menu, "git.menu.get_from_revision", UiIcon::DOWNLOAD);
    let menu = menu.item(PopupMenuItem::separator());
    let menu = append_history_item(menu, workspace.clone(), &full_path);
    let menu = append_diff_action(
        menu,
        workspace.clone(),
        &full_path,
        "git.menu.show_parent_changes",
        UiIcon::DIFF,
    );
    append_agent_context(menu, workspace, &full_path)
}

fn append_diff_action(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    path: &str,
    label_key: &'static str,
    icon: UiIcon,
) -> PopupMenu {
    let path = path.to_string();
    menu.item(menu_item(
        rust_i18n::t!(label_key).to_string(),
        AppIcon::Ui(icon),
        move |_, _, cx| {
            let path = path.clone();
            workspace.update(cx, |ws, cx| ws.diff_commit_file(&path, cx));
        },
    ))
}

fn append_edit_source(menu: PopupMenu, workspace: Entity<GitWorkspace>, path: &str) -> PopupMenu {
    let path = path.to_string();
    menu.item(menu_item(
        rust_i18n::t!("git.menu.edit_source").to_string(),
        AppIcon::Ui(UiIcon::DIFF),
        move |_, _, cx| {
            let path = path.clone();
            workspace.update(cx, |ws, cx| {
                let full_file_path = ws.state.project.path.join(&path);
                ws.copy_to_clipboard(&full_file_path.to_string_lossy(), cx);
            });
        },
    ))
}

fn append_rollback_action(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    path: &str,
    label_key: &'static str,
    icon: UiIcon,
) -> PopupMenu {
    let path = path.to_string();
    menu.item(
        MenuItem::new(rust_i18n::t!(label_key).to_string(), AppIcon::Ui(icon)).render(
            move |_, _, cx| {
                let path = path.clone();
                workspace.update(cx, |ws, cx| ws.rollback_single_path(&path, cx));
            },
        ),
    )
}

fn append_disabled_item(menu: PopupMenu, label_key: &'static str, icon: UiIcon) -> PopupMenu {
    menu.item(
        MenuItem::new(rust_i18n::t!(label_key).to_string(), AppIcon::Ui(icon))
            .disabled(true)
            .render(|_, _, _| {}),
    )
}

fn append_create_patch(menu: PopupMenu, workspace: Entity<GitWorkspace>, path: &str) -> PopupMenu {
    let path = path.to_string();
    menu.item(menu_item(
        rust_i18n::t!("git.menu.create_patch").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let path = path.clone();
            workspace.update(cx, |ws, cx| ws.create_patch(&path, cx));
        },
    ))
}

fn append_history_item(menu: PopupMenu, workspace: Entity<GitWorkspace>, path: &str) -> PopupMenu {
    let path = path.to_string();
    menu.item(menu_item(
        rust_i18n::t!("git.menu.history_up_to_here").to_string(),
        AppIcon::Ui(UiIcon::HISTORY),
        move |_, _, cx| {
            let path = path.clone();
            workspace.update(cx, |ws, cx| ws.set_log_path(Some(path), cx));
        },
    ))
}

fn append_agent_context(menu: PopupMenu, workspace: Entity<GitWorkspace>, path: &str) -> PopupMenu {
    let path = path.to_string();
    menu.item(menu_item(
        rust_i18n::t!("git.menu.add_to_agent_context").to_string(),
        AppIcon::Ui(UiIcon::AI_INTENTION),
        move |_, _, cx| {
            let path = path.clone();
            workspace.update(cx, |ws, cx| ws.copy_to_clipboard(&format!("@{path}"), cx));
        },
    ))
}
