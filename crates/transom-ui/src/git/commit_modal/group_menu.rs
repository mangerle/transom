//! 分组头右键菜单：整组暂存 / 取消暂存 / 删除 / 回滚。
//!
//! 设计原理（Why）：整组动作的文案与执行集中在这里；忽略组没有整组动作，由
//! `ChangeGroup::bulk_action` 返回 None 兜住，调用方不挂菜单。

use super::ChangeGroup;
use crate::git::GitWorkspace;
use crate::widgets::menu_item;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use transom_core::GitAction;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 分组头右键菜单：整组暂存/取消暂存/删除与整组回滚。
///
/// 忽略组没有整组动作，调用方不会给它挂菜单；这里的 `None` 分支只保证
/// 将来有人漏加判断时也是一个空菜单而不是一句错误的动作。
pub(crate) fn render_group_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    group: ChangeGroup,
    paths: &[String],
) -> PopupMenu {
    let Some(action) = group.bulk_action() else {
        return menu;
    };
    let paths = paths.to_vec();
    let stage_ws = workspace.clone();
    let discard_ws = workspace;
    let stage_paths = paths.clone();
    let discard_paths = paths;
    menu.item(menu_item(
        rust_i18n::t!(action_label(action)).to_string(),
        AppIcon::Ui(UiIcon::ADD),
        move |_, _, cx| {
            run_group_action(&stage_ws, action, &stage_paths, cx);
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.discard_all").to_string(),
        AppIcon::Ui(UiIcon::UNDO),
        move |_, _, cx| {
            run_group_action(&discard_ws, GitAction::Discard, &discard_paths, cx);
        },
    ))
}

fn action_label(action: GitAction) -> &'static str {
    match action {
        GitAction::Stage => "git.menu.stage_all",
        GitAction::Unstage => "git.menu.unstage_all",
        GitAction::DeleteUntracked => "git.menu.delete_all",
        other => other.i18n_key(),
    }
}

/// 整组动作：按路径批量执行。
fn run_group_action(
    workspace: &Entity<GitWorkspace>,
    action: GitAction,
    paths: &[String],
    cx: &mut App,
) {
    let paths = paths.to_vec();
    workspace.update(cx, |ws, cx| {
        ws.run_paths(action, paths, cx);
    });
}
