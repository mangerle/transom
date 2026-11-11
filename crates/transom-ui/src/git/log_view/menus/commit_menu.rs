//! 提交表格行右键上下文菜单（全方位对齐 IntelliJ IDEA 提交上下文菜单）。
//!
//! 设计原理（Why）：
//! - 顺序、分组与文案对齐 IDEA；
//! - 菜单操作不持有界面的可变引用，通过目标包 `CommitMenuTarget` 解耦；
//! - 包含向智能体上下文添加、拣选、还原、重置、交互式变基、新建分支/标记与父子跳转。

use crate::app::AppShell;
use crate::git::types::LogJump;
use crate::widgets::{MenuItem, menu_item};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{GitProject, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 提交右键菜单的目标包。
pub(crate) struct CommitMenuTarget {
    pub(crate) project: GitProject,
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
        project,
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
        // 1. 复制修订号
        .item(menu_item(
            rust_i18n::t!("git.menu.copy_oid").to_string(),
            AppIcon::Ui(UiIcon::COPY),
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
            AppIcon::Ui(UiIcon::AI_INTENTION),
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
            AppIcon::Ui(UiIcon::DOWNLOAD),
            {
                let patch_app = app.clone();
                let patch_project = project.clone();
                move |_, _, cx| {
                    let oid = patch_oid.clone();
                    let project = patch_project.clone();
                    patch_app.update(cx, |this, cx| {
                        this.git_create_patch(&project, &oid, cx);
                    });
                }
            },
        ))
        // 4. 优选 (Cherry-pick)
        .item(menu_item(
            rust_i18n::t!("git.menu.cherry_pick").to_string(),
            AppIcon::Ui(UiIcon::COMMIT),
            {
                let pick_app = app.clone();
                let pick_project = project.clone();
                move |_, _, cx| {
                    let oid = pick_oid.clone();
                    let project = pick_project.clone();
                    pick_app.update(cx, |this, cx| {
                        this.git_history_on_project(
                            &project,
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
            AppIcon::Ui(UiIcon::VCS),
            {
                let checkout_app = app.clone();
                let checkout_project = project.clone();
                move |_, _, cx| {
                    let oid = checkout_oid.clone();
                    let project = checkout_project.clone();
                    checkout_app.update(cx, |this, cx| {
                        this.git_checkout_project_ref(&project, &oid, cx);
                    });
                }
            },
        ))
        // 6. 在修订版中显示仓库
        .item(menu_item(
            rust_i18n::t!("git.menu.browse_repo_at_revision").to_string(),
            AppIcon::Ui(UiIcon::FOLDER),
            {
                let browse_app = app.clone();
                let browse_project = project.clone();
                let browse_oid = oid.clone();
                move |_, _, cx| {
                    let oid = browse_oid.clone();
                    let project = browse_project.clone();
                    browse_app.update(cx, |this, cx| {
                        this.git_browse_commit(&project, &oid, cx);
                    });
                }
            },
        ))
        // 7. 与本地比较
        .item(menu_item(
            rust_i18n::t!("git.menu.compare_with_local").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
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
            AppIcon::Ui(UiIcon::UNDO),
            {
                let reset_app = app.clone();
                let reset_project = project.clone();
                let target = reset_oid.clone();
                move |_, _, cx| {
                    let target = target.clone();
                    let project = reset_project.clone();
                    reset_app.update(cx, |this, cx| {
                        this.git_history_on_project(
                            &project,
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
            AppIcon::Ui(UiIcon::UNDO),
            {
                let revert_app = app.clone();
                let revert_project = project.clone();
                move |_, _, cx| {
                    let oid = revert_oid.clone();
                    let project = revert_project.clone();
                    revert_app.update(cx, |this, cx| {
                        this.git_history_on_project(
                            &project,
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
                AppIcon::Ui(UiIcon::UNDO),
            )
            .disabled(!on_branch)
            .render({
                let undo_app = app.clone();
                let undo_project = project.clone();
                let undo_oid = oid.clone();
                move |_, _, cx| {
                    let target = undo_oid.clone();
                    let project = undo_project.clone();
                    undo_app.update(cx, |this, cx| {
                        this.git_history_on_project(
                            &project,
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
        // 11. 编辑提交信息...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.edit_commit_message").to_string(),
                AppIcon::Ui(UiIcon::EDIT),
            )
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
                AppIcon::Ui(UiIcon::DOWNLOAD),
            )
            .disabled(!can_fold)
            .render({
                let fold_app = app.clone();
                let fold_project = project.clone();
                move |_, _, cx| {
                    let project = fold_project.clone();
                    fold_app.update(cx, |this, cx| {
                        this.git_fold_tip_into_parent(&project, false, cx);
                    });
                }
            }),
        )
        // 13. 压缩到...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.squash_parent").to_string(),
                AppIcon::Ui(UiIcon::DOWN),
            )
            .disabled(!can_fold)
            .render({
                let fold_app = app.clone();
                let fold_project = project.clone();
                move |_, _, cx| {
                    let project = fold_project.clone();
                    fold_app.update(cx, |this, cx| {
                        this.git_fold_tip_into_parent(&project, true, cx);
                    });
                }
            }),
        )
        // 14. 删除提交
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.drop_commit").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
            )
            .disabled(!can_fold)
            .render({
                let drop_app = app.clone();
                let drop_project = project.clone();
                move |_, _, cx| {
                    let project = drop_project.clone();
                    drop_app.update(cx, |this, cx| {
                        this.git_history_on_project(
                            &project,
                            crate::git::actions::GitHistoryAction::Reset {
                                target: "HEAD~1".to_string(),
                                kind: transom_core::GitResetKind::Hard,
                            },
                            cx,
                        );
                    });
                }
            }),
        )
        // 15. 从这里进行交互式变基...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.rebase_from_here").to_string(),
                AppIcon::Ui(UiIcon::MERGE),
            )
            .disabled(is_tip)
            .render({
                let rebase_app = app.clone();
                let rebase_project = project.clone();
                let rebase_oid = oid.clone();
                move |_, _, cx| {
                    let project = rebase_project.clone();
                    let oid = rebase_oid.clone();
                    rebase_app.update(cx, |this, cx| {
                        this.git_open_rebase_modal(&project, &oid, cx);
                    });
                }
            }),
        )
        // 16. 推送此前所有提交...
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.push_up_to").to_string(),
                AppIcon::Ui(UiIcon::UPLOAD),
            )
            .disabled(!on_branch)
            .render({
                let push_app = app.clone();
                let push_project = project.clone();
                let push_oid = oid.clone();
                move |_, _, cx| {
                    let project = push_project.clone();
                    let oid = push_oid.clone();
                    push_app.update(cx, |this, cx| {
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
            AppIcon::Ui(UiIcon::DIFF),
            {
                let rebase_app = app.clone();
                let rebase_project = project.clone();
                move |_, _, cx| {
                    let project = rebase_project.clone();
                    let upstream = rebase_onto_oid.clone();
                    rebase_app.update(cx, |this, cx| {
                        this.git_history_on_project(
                            &project,
                            crate::git::actions::GitHistoryAction::Rebase(upstream),
                            cx,
                        );
                    });
                }
            },
        ))
        // 18. 新建分支...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_branch_from").to_string(),
            AppIcon::Ui(UiIcon::VCS),
            {
                let branch_app = app.clone();
                let branch_project = project.clone();
                move |_, window, cx| {
                    let project = branch_project.clone();
                    let oid = branch_oid.clone();
                    branch_app.update(cx, |this, cx| {
                        this.git_create_project_branch_from(&project, &oid, window, cx);
                    });
                }
            },
        ))
        // 19. 新建标记...
        .item(menu_item(
            rust_i18n::t!("git.menu.new_tag_from").to_string(),
            AppIcon::Ui(UiIcon::TAG),
            {
                let tag_app = app.clone();
                let tag_project = project;
                move |_, window, cx| {
                    let project = tag_project.clone();
                    let oid = tag_oid.clone();
                    tag_app.update(cx, |this, cx| {
                        this.git_create_project_tag_from(&project, &oid, window, cx);
                    });
                }
            },
        ))
        .item(PopupMenuItem::separator())
        // 20. 转到子提交 (向左箭头)
        .item(menu_item(
            rust_i18n::t!("git.menu.jump_child").to_string(),
            AppIcon::Ui(UiIcon::DOWNLOAD),
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
                AppIcon::Ui(UiIcon::UPLOAD),
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
