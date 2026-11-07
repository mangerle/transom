//! 提交表格行右键上下文菜单（全方位对齐 IntelliJ IDEA 提交上下文菜单）。
//!
//! 设计原理（Why）：
//! - 顺序、分组、文案与快捷键完整对齐 IDEA；
//! - 菜单操作不持有界面的可变引用，通过目标包 `CommitMenuTarget` 解耦；
//! - 包含向智能体上下文添加、拣选、还原、重置、交互式变基、新建分支/标记与父子跳转。

use crate::app::AppShell;
use crate::git::types::LogJump;
use crate::widgets::{MenuItem, menu_item, menu_item_with_shortcut};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::TabId;

/// 提交右键菜单的目标包。
pub(crate) struct CommitMenuTarget {
    pub(crate) tab_id: TabId,
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
    app: Entity<AppShell>,
    target: CommitMenuTarget,
) -> PopupMenu {
    let CommitMenuTarget {
        tab_id,
        oid,
        parents,
        on_branch,
        can_fold,
        is_tip,
        current_branch,
    } = target;

    let branch_oid = oid.clone();
    let tag_oid = oid.clone();
    let revert_oid = oid.clone();
    let pick_oid = oid.clone();
    let copy_oid = oid.clone();
    let patch_oid = oid.clone();
    let checkout_oid = oid.clone();
    let compare_oid = oid.clone();
    let ai_oid = oid.clone();
    let reset_oid = oid.clone();
    let jump_oid = oid.clone();
    let rebase_onto_oid = oid.clone();

    let cur_branch_name = current_branch.unwrap_or_else(|| "HEAD".to_string());

    menu
        // 1. 复制修订号 (Ctrl+Alt+Shift+C)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.copy_oid").to_string(),
            IconName::Copy,
            "Ctrl+Alt+Shift+C",
            {
                let copy_app = app.clone();
                move |_, _, cx| {
                    let oid = copy_oid.clone();
                    copy_app.update(cx, |this, cx| {
                        this.write_clipboard(&oid, cx);
                    });
                }
            },
        ))
        // 2. 添加到智能体上下文
        .item(menu_item(
            rust_i18n::t!("git.menu.add_to_agent_context").to_string(),
            IconName::Sparkles,
            {
                let ai_app = app.clone();
                move |_, _, cx| {
                    let oid = ai_oid.clone();
                    ai_app.update(cx, |this, cx| {
                        this.write_clipboard(&format!("commit {oid}"), cx);
                    });
                }
            },
        ))
        // 3. 创建补丁...
        .item(menu_item(
            rust_i18n::t!("git.menu.create_patch").to_string(),
            IconName::Download,
            {
                let patch_app = app.clone();
                move |_, _, cx| {
                    let oid = patch_oid.clone();
                    patch_app.update(cx, |this, cx| {
                        let Some(project) = this.git_project_of_tab(Some(tab_id)) else {
                            return;
                        };
                        this.git_create_patch(&project, &oid, cx);
                    });
                }
            },
        ))
        // 4. 优选 (Cherry-pick)
        .item(menu_item(
            rust_i18n::t!("git.menu.cherry_pick").to_string(),
            IconName::Cherry,
            {
                let pick_app = app.clone();
                move |_, _, cx| {
                    let oid = pick_oid.clone();
                    pick_app.update(cx, |this, cx| {
                        this.git_history_on_current(
                            crate::git::actions::GitHistoryAction::CherryPick(oid),
                            cx,
                        );
                    });
                }
            },
        ))
        .item(PopupMenuItem::separator())
        // 5. 签出修订
        .item(menu_item(
            rust_i18n::t!("git.menu.checkout_revision").to_string(),
            IconName::GitBranch,
            {
                let checkout_app = app.clone();
                move |_, _, cx| {
                    let oid = checkout_oid.clone();
                    checkout_app.update(cx, |this, cx| {
                        this.git_checkout_ref(&oid, cx);
                    });
                }
            },
        ))
        // 6. 在修订版中显示仓库
        .item(menu_item(
            rust_i18n::t!("git.menu.browse_repo_at_revision").to_string(),
            IconName::FolderGit,
            {
                let browse_app = app.clone();
                let browse_oid = oid.clone();
                move |_, _, cx| {
                    let oid = browse_oid.clone();
                    browse_app.update(cx, |this, cx| {
                        let Some(project) = this.git_project_of_tab(Some(tab_id)) else {
                            return;
                        };
                        this.git_browse_commit(&project, &oid, cx);
                    });
                }
            },
        ))
        // 7. 与本地比较
        .item(menu_item(
            rust_i18n::t!("git.menu.compare_with_local").to_string(),
            IconName::GitCompareArrows,
            {
                let compare_app = app.clone();
                move |_, _, cx| {
                    let oid = compare_oid.clone();
                    compare_app.update(cx, |this, cx| {
                        this.git_diff_commit_file(tab_id, &oid, cx);
                    });
                }
            },
        ))
        .item(PopupMenuItem::separator())
        // 8. 将当前分支重置到此处... (三档快捷重置)
        .item(menu_item(
            rust_i18n::t!("git.menu.reset_current_branch_here").to_string(),
            IconName::RotateCcw,
            {
                let reset_app = app.clone();
                let target = reset_oid.clone();
                move |_, _, cx| {
                    let target = target.clone();
                    reset_app.update(cx, |this, cx| {
                        this.git_history_on_current(
                            crate::git::actions::GitHistoryAction::Reset {
                                target,
                                kind: transom_core::GitResetKind::Mixed,
                            },
                            cx,
                        );
                    });
                }
            },
        ))
        // 9. 还原提交 (Revert)
        .item(menu_item(
            rust_i18n::t!("git.menu.revert_commit").to_string(),
            IconName::Undo2,
            {
                let revert_app = app.clone();
                move |_, _, cx| {
                    let oid = revert_oid.clone();
                    revert_app.update(cx, |this, cx| {
                        this.git_history_on_current(
                            crate::git::actions::GitHistoryAction::Revert(oid),
                            cx,
                        );
                    });
                }
            },
        ))
        // 10. 撤销提交...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.undo_commit").to_string(),
                IconName::RotateCcw,
            )
            .disabled(!on_branch)
            .render({
                let undo_app = app.clone();
                let undo_oid = oid.clone();
                move |_, _, cx| {
                    let target = undo_oid.clone();
                    undo_app.update(cx, |this, cx| {
                        this.git_history_on_current(
                            crate::git::actions::GitHistoryAction::Reset {
                                target,
                                kind: transom_core::GitResetKind::Soft,
                            },
                            cx,
                        );
                    });
                }
            }),
        )
        .item(PopupMenuItem::separator())
        // 11. 编辑提交信息... (F2)
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.edit_commit_message").to_string(),
                IconName::Pencil,
            )
            .shortcut("F2")
            .disabled(!can_fold)
            .render({
                let edit_app = app.clone();
                move |_, window, cx| {
                    edit_app.update(cx, |this, cx| {
                        this.git_toggle_commit_amend(tab_id, window, cx);
                    });
                }
            }),
        )
        // 12. Fixup...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.fixup_parent").to_string(),
                IconName::ArrowDownToLine,
            )
            .disabled(!can_fold)
            .render({
                let fold_app = app.clone();
                move |_, _, cx| {
                    fold_app.update(cx, |this, cx| {
                        let Some(project) = this.git_project_of_tab(Some(tab_id)) else {
                            return;
                        };
                        this.git_fold_tip_into_parent(&project, false, cx);
                    });
                }
            }),
        )
        // 13. 压缩到...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.squash_parent").to_string(),
                IconName::ArrowDown,
            )
            .disabled(!can_fold)
            .render({
                let fold_app = app.clone();
                move |_, _, cx| {
                    fold_app.update(cx, |this, cx| {
                        let Some(project) = this.git_project_of_tab(Some(tab_id)) else {
                            return;
                        };
                        this.git_fold_tip_into_parent(&project, true, cx);
                    });
                }
            }),
        )
        // 14. 删除提交
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.drop_commit").to_string(),
                IconName::Trash,
            )
            .disabled(!can_fold)
            .render(|_, _, _| {}),
        )
        // 15. 从这里进行交互式变基...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.rebase_from_here").to_string(),
                IconName::GitPullRequest,
            )
            .disabled(is_tip)
            .render({
                let rebase_app = app.clone();
                let rebase_oid = oid.clone();
                move |_, _, cx| {
                    let oid = rebase_oid.clone();
                    rebase_app.update(cx, |this, cx| {
                        let Some(project) = this.git_project_of_tab(Some(tab_id)) else {
                            return;
                        };
                        this.git_open_rebase_modal(&project, &oid, cx);
                    });
                }
            }),
        )
        // 16. 推送此前所有提交...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.push_up_to").to_string(),
                IconName::Upload,
            )
            .disabled(!on_branch)
            .render({
                let push_app = app.clone();
                let push_oid = oid.clone();
                move |_, _, cx| {
                    let oid = push_oid.clone();
                    push_app.update(cx, |this, cx| {
                        let Some(project) = this.git_project_of_tab(Some(tab_id)) else {
                            return;
                        };
                        this.git_push_up_to(&project, &oid, cx);
                    });
                }
            }),
        )
        .item(PopupMenuItem::separator())
        // 17. 将 '{branch}' 变基到所选提交
        .item(menu_item(
            rust_i18n::t!(
                "git.menu.rebase_current_onto_selected",
                branch = cur_branch_name.as_str()
            )
            .to_string(),
            IconName::GitCompareArrows,
            {
                let rebase_app = app.clone();
                move |_, _, cx| {
                    let upstream = rebase_onto_oid.clone();
                    rebase_app.update(cx, |this, cx| {
                        this.git_history_on_current(
                            crate::git::actions::GitHistoryAction::Rebase(upstream),
                            cx,
                        );
                    });
                }
            },
        ))
        // 18. 新建分支... (Ctrl+Alt+N)
        .item(menu_item_with_shortcut(
            rust_i18n::t!("git.menu.new_branch_from").to_string(),
            IconName::GitBranchPlus,
            "Ctrl+Alt+N",
            {
                let branch_app = app.clone();
                move |_, window, cx| {
                    let oid = branch_oid.clone();
                    branch_app.update(cx, |this, cx| {
                        this.git_create_branch_from(&oid, window, cx);
                    });
                }
            },
        ))
        // 19. 新建标记...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_tag_from").to_string(),
            IconName::Tag,
            {
                let tag_app = app.clone();
                move |_, window, cx| {
                    let oid = tag_oid.clone();
                    tag_app.update(cx, |this, cx| {
                        this.git_create_tag_from(&oid, window, cx);
                    });
                }
            },
        ))
        .item(PopupMenuItem::separator())
        // 20. 转到子提交 (向左箭头)
        .item(menu_item(
            rust_i18n::t!("git.menu.jump_child").to_string(),
            IconName::ArrowDownToLine,
            {
                let jump_app = app.clone();
                let target = jump_oid.clone();
                move |_, _, cx| {
                    let target = target.clone();
                    jump_app.update(cx, |this, cx| {
                        this.git_jump_commit(tab_id, LogJump::Child(target), cx);
                    });
                }
            },
        ))
        // 21. 转到父提交 (向右箭头)
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.jump_parent").to_string(),
                IconName::ArrowUpToLine,
            )
            .disabled(parents.is_empty())
            .render({
                let jump_app = app.clone();
                let target = jump_oid;
                move |_, _, cx| {
                    let target = target.clone();
                    jump_app.update(cx, |this, cx| {
                        this.git_jump_commit(tab_id, LogJump::Parent(target), cx);
                    });
                }
            }),
        )
}
