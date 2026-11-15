//! 提交表格行右键上下文菜单（全方位对齐 IntelliJ IDEA 提交上下文菜单）。
//!
//! 设计原理（Why）：
//! - 顺序、分组与文案对齐 IDEA；
//! - 菜单操作直接落在所属 `Entity<GitWorkspace>` 上，跨窗口动作经实体事件上报，彻底解耦外壳；
//! - 按功能组拆分为 meta / history / ref_jump 子构建函数，避免单体超长渲染函数。

use crate::git::GitWorkspace;
use crate::git::actions::GitHistoryAction;
use crate::git::types::LogJump;
use crate::widgets::{MenuItem, menu_item};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::GitResetKind;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 提交右键菜单的目标包。
pub(crate) struct CommitMenuTarget {
    pub(crate) oid: String,
    pub(crate) parents: Vec<String>,
    pub(crate) on_branch: bool,
    /// 这一行是否可以被「压缩到／优选到父提交」。
    pub(crate) can_fold: bool,
    /// 这一行是否就是分支头提交：它之上没有可重排的提交，不能作为变基基线。
    pub(crate) is_tip: bool,
    /// 当前活跃分支名（用于动态格式化菜单文本）。
    pub(crate) current_branch: Option<String>,
}

/// 提交行右键菜单：完全对齐 IDEA 提交菜单布局。
pub(crate) fn render_commit_menu(
    menu: PopupMenu,
    workspace: Entity<GitWorkspace>,
    target: CommitMenuTarget,
) -> PopupMenu {
    let cur_branch_name = target
        .current_branch
        .as_deref()
        .unwrap_or("HEAD")
        .to_string();
    let menu = append_commit_meta_items(menu, &workspace, &target.oid);
    let menu = append_commit_history_items(menu, &workspace, &target, &cur_branch_name);
    append_commit_ref_and_jump_items(menu, &workspace, &target.oid, !target.parents.is_empty())
}

fn append_commit_meta_items(
    menu: PopupMenu,
    workspace: &Entity<GitWorkspace>,
    oid: &str,
) -> PopupMenu {
    let copy_ws = workspace.clone();
    let ai_ws = workspace.clone();
    let patch_ws = workspace.clone();
    let pick_ws = workspace.clone();
    let checkout_ws = workspace.clone();
    let browse_ws = workspace.clone();
    let compare_ws = workspace.clone();

    let copy_oid = oid.to_string();
    let ai_oid = oid.to_string();
    let patch_oid = oid.to_string();
    let pick_oid = oid.to_string();
    let checkout_oid = oid.to_string();
    let browse_oid = oid.to_string();
    let compare_oid = oid.to_string();

    menu.item(menu_item(
        rust_i18n::t!("git.menu.copy_oid").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        move |_, _, cx| {
            let oid = copy_oid.clone();
            copy_ws.update(cx, |ws, cx| ws.copy_to_clipboard(&oid, cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.add_to_agent_context").to_string(),
        AppIcon::Ui(UiIcon::AI_INTENTION),
        move |_, _, cx| {
            let oid = ai_oid.clone();
            ai_ws.update(cx, |ws, cx| {
                ws.copy_to_clipboard(&format!("commit {oid}"), cx)
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.create_patch").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let oid = patch_oid.clone();
            patch_ws.update(cx, |ws, cx| ws.create_patch(&oid, cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.cherry_pick").to_string(),
        AppIcon::Ui(UiIcon::COMMIT),
        move |_, _, cx| {
            let oid = pick_oid.clone();
            pick_ws.update(cx, |ws, cx| {
                ws.history_action(GitHistoryAction::CherryPick(oid), cx);
            });
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.menu.checkout_revision").to_string(),
        AppIcon::Ui(UiIcon::VCS),
        move |_, _, cx| {
            let oid = checkout_oid.clone();
            checkout_ws.update(cx, |ws, cx| ws.checkout_ref(&oid, cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.browse_repo_at_revision").to_string(),
        AppIcon::Ui(UiIcon::FOLDER),
        move |_, _, cx| {
            let oid = browse_oid.clone();
            browse_ws.update(cx, |ws, cx| ws.browse_commit(&oid, cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.compare_with_local").to_string(),
        AppIcon::Ui(UiIcon::DIFF),
        move |_, _, cx| {
            let oid = compare_oid.clone();
            compare_ws.update(cx, |ws, cx| ws.diff_commit_file(&oid, cx));
        },
    ))
    .item(PopupMenuItem::separator())
}

fn append_commit_history_items(
    menu: PopupMenu,
    workspace: &Entity<GitWorkspace>,
    target: &CommitMenuTarget,
    cur_branch_name: &str,
) -> PopupMenu {
    let reset_ws = workspace.clone();
    let revert_ws = workspace.clone();
    let undo_ws = workspace.clone();
    let edit_ws = workspace.clone();
    let fixup_ws = workspace.clone();
    let squash_ws = workspace.clone();
    let drop_ws = workspace.clone();
    let rebase_modal_ws = workspace.clone();
    let push_ws = workspace.clone();
    let rebase_onto_ws = workspace.clone();

    let reset_oid = target.oid.clone();
    let revert_oid = target.oid.clone();
    let undo_oid = target.oid.clone();
    let rebase_modal_oid = target.oid.clone();
    let push_oid = target.oid.clone();
    let rebase_onto_oid = target.oid.clone();
    let on_branch = target.on_branch;
    let can_fold = target.can_fold;
    let is_tip = target.is_tip;

    menu.item(menu_item(
        rust_i18n::t!("git.menu.reset_current_branch_here").to_string(),
        AppIcon::Ui(UiIcon::UNDO),
        move |_, _, cx| {
            let target = reset_oid.clone();
            reset_ws.update(cx, |ws, cx| {
                ws.history_action(
                    GitHistoryAction::Reset {
                        target,
                        kind: GitResetKind::Mixed,
                    },
                    cx,
                );
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.revert_commit").to_string(),
        AppIcon::Ui(UiIcon::UNDO),
        move |_, _, cx| {
            let oid = revert_oid.clone();
            revert_ws.update(cx, |ws, cx| {
                ws.history_action(GitHistoryAction::Revert(oid), cx);
            });
        },
    ))
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.undo_commit").to_string(),
            AppIcon::Ui(UiIcon::UNDO),
        )
        .disabled(!on_branch)
        .render(move |_, _, cx| {
            let target = undo_oid.clone();
            undo_ws.update(cx, |ws, cx| {
                ws.history_action(
                    GitHistoryAction::Reset {
                        target,
                        kind: GitResetKind::Soft,
                    },
                    cx,
                );
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.edit_commit_message").to_string(),
            AppIcon::Ui(UiIcon::EDIT),
        )
        .disabled(!can_fold)
        .render(move |_, window, cx| {
            edit_ws.update(cx, |ws, cx| ws.toggle_commit_amend(window, cx));
        }),
    )
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.fixup_parent").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
        )
        .disabled(!can_fold)
        .render(move |_, _, cx| {
            fixup_ws.update(cx, |ws, cx| ws.fold_tip_into_parent(false, cx));
        }),
    )
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.squash_parent").to_string(),
            AppIcon::Ui(UiIcon::DOWN),
        )
        .disabled(!can_fold)
        .render(move |_, _, cx| {
            squash_ws.update(cx, |ws, cx| ws.fold_tip_into_parent(true, cx));
        }),
    )
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.drop_commit").to_string(),
            AppIcon::Ui(UiIcon::DELETE),
        )
        .disabled(!can_fold)
        .render(move |_, _, cx| {
            drop_ws.update(cx, |ws, cx| {
                ws.history_action(
                    GitHistoryAction::Reset {
                        target: "HEAD~1".to_string(),
                        kind: GitResetKind::Hard,
                    },
                    cx,
                );
            });
        }),
    )
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.rebase_from_here").to_string(),
            AppIcon::Ui(UiIcon::MERGE),
        )
        .disabled(is_tip)
        .render(move |_, _, cx| {
            let upstream = rebase_modal_oid.clone();
            rebase_modal_ws.update(cx, |ws, cx| ws.open_rebase_modal(upstream, cx));
        }),
    )
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.push_up_to").to_string(),
            AppIcon::Ui(UiIcon::UPLOAD),
        )
        .disabled(!on_branch)
        .render(move |_, _, cx| {
            let oid = push_oid.clone();
            push_ws.update(cx, |ws, cx| ws.push_up_to(&oid, cx));
        }),
    )
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!(
            "git.menu.rebase_current_onto_selected",
            branch = cur_branch_name
        )
        .to_string(),
        AppIcon::Ui(UiIcon::DIFF),
        move |_, _, cx| {
            let upstream = rebase_onto_oid.clone();
            rebase_onto_ws.update(cx, |ws, cx| {
                ws.history_action(GitHistoryAction::Rebase(upstream), cx);
            });
        },
    ))
}

fn append_commit_ref_and_jump_items(
    menu: PopupMenu,
    workspace: &Entity<GitWorkspace>,
    oid: &str,
    has_parents: bool,
) -> PopupMenu {
    let branch_ws = workspace.clone();
    let tag_ws = workspace.clone();
    let child_ws = workspace.clone();
    let parent_ws = workspace.clone();

    let branch_oid = oid.to_string();
    let tag_oid = oid.to_string();
    let child_oid = oid.to_string();
    let parent_oid = oid.to_string();

    menu.item(menu_item(
        rust_i18n::t!("git.menu.new_branch_from").to_string(),
        AppIcon::Ui(UiIcon::VCS),
        move |_, _, cx| {
            let start = branch_oid.clone();
            branch_ws.update(cx, |ws, cx| ws.create_branch_from(Some(start), cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.new_tag_from").to_string(),
        AppIcon::Ui(UiIcon::TAG),
        move |_, _, cx| {
            let oid = tag_oid.clone();
            tag_ws.update(cx, |ws, cx| ws.create_tag_from(&oid, cx));
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.menu.jump_child").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let target = child_oid.clone();
            child_ws.update(cx, |ws, cx| ws.jump_commit(LogJump::Child(target), cx));
        },
    ))
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.jump_parent").to_string(),
            AppIcon::Ui(UiIcon::UPLOAD),
        )
        .disabled(!has_parents)
        .render(move |_, _, cx| {
            let target = parent_oid.clone();
            parent_ws.update(cx, |ws, cx| ws.jump_commit(LogJump::Parent(target), cx));
        }),
    )
}
