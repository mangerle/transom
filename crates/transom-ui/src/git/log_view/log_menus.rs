//! 日志工作区的右键菜单集合：过滤条的日期/路径、引用栏的分支与标签、
//! 提交行的整组动作。
//!
//! 设计原理（Why）：菜单项是「与 IDEA 同序」这一口径最集中的地方，单独成模块
//! 便于逐项对照截图核查；每个菜单都只依赖实体句柄与目标包，不持有渲染期的状态借用。

use crate::app::AppShell;
use crate::git::types::LogJump;
use crate::widgets::{MenuItem, menu_item};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::TabId;

/// 日期范围菜单。
pub(crate) fn render_date_menu(menu: PopupMenu, app: Entity<AppShell>, tab_id: TabId) -> PopupMenu {
    let presets: [(&str, Option<&str>); 4] = [
        ("git.filter.date_all", None),
        ("git.filter.date_today", Some("today")),
        ("git.filter.date_week", Some("1 week ago")),
        ("git.filter.date_month", Some("1 month ago")),
    ];
    let mut menu = menu;
    for (key, value) in presets {
        let item_app = app.clone();
        let owned = value.map(str::to_string);
        menu = menu.item(PopupMenuItem::new(rust_i18n::t!(key).to_string()).on_click(
            move |_, _, cx| {
                let since = owned.clone();
                item_app.update(cx, |this, cx| {
                    this.git_set_log_dates(tab_id, since, cx);
                });
            },
        ));
    }
    menu
}

/// 路径过滤菜单：列出跟踪文件（上限 300，超出时提示用检索框）。
pub(crate) fn render_path_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    tab_id: TabId,
    paths: Vec<String>,
) -> PopupMenu {
    let all_app = app.clone();
    let menu = menu.item(
        PopupMenuItem::new(rust_i18n::t!("git.filter.path_all").to_string()).on_click(
            move |_, _, cx| {
                all_app.update(cx, |this, cx| this.git_set_log_path(tab_id, None, cx));
            },
        ),
    );
    if paths.is_empty() {
        return menu.item(
            PopupMenuItem::new(rust_i18n::t!("git.filter.loading_paths").to_string())
                .disabled(true),
        );
    }
    let mut menu = menu;
    for path in paths.into_iter().take(300) {
        let item_app = app.clone();
        menu = menu.item(menu_item(path.clone(), IconName::File, move |_, _, cx| {
            let path = path.clone();
            item_app.update(cx, |this, cx| {
                this.git_set_log_path(tab_id, Some(path), cx);
            });
        }));
    }
    menu
}

/// 引用右键菜单（对应 IDEA 的分支/标签上下文菜单）。
pub(crate) fn render_ref_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    name: String,
    renameable: bool,
) -> PopupMenu {
    let app_checkout = app.clone();
    let app_branch = app.clone();
    let app_pull = app.clone();
    let app_push = app.clone();
    let app_worktree = app.clone();
    let branch_name = name.clone();
    let worktree_name = name.clone();
    let delete_name = name.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.menu.checkout").to_string(),
        IconName::ArrowRight,
        {
            let checkout_name = name.clone();
            move |_, _, cx| {
                let name = checkout_name.clone();
                app_checkout.update(cx, |this, cx| {
                    this.git_checkout_ref(&name, cx);
                });
            }
        },
    ))
    .item(
        MenuItem::new(
            rust_i18n::t!("git.menu.rename_branch").to_string(),
            IconName::Pencil,
        )
        .disabled(!renameable)
        .render({
            let rename_app = app.clone();
            let rename_old = name.clone();
            move |_, window, cx| {
                let old = rename_old.clone();
                rename_app.update(cx, |this, cx| {
                    let Some(project) = this.git_project_of_tab(None) else {
                        return;
                    };
                    this.open_git_name_modal(
                        &project,
                        crate::git::GitNamePurpose::RenameBranch { old: old.clone() },
                        old,
                        window,
                        cx,
                    );
                });
            }
        }),
    )
    .item(menu_item(
        rust_i18n::t!("git.menu.new_branch_from").to_string(),
        IconName::GitBranchPlus,
        move |_, window, cx| {
            let start = branch_name.clone();
            app_branch.update(cx, |this, cx| {
                this.git_create_branch_from(&start, window, cx);
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.merge_into").to_string(),
        IconName::GitMerge,
        {
            let merge_app = app.clone();
            let merge_name = name.clone();
            move |_, _, cx| {
                let reference = merge_name.clone();
                merge_app.update(cx, |this, cx| {
                    this.git_history_on_current(
                        crate::git::actions::GitHistoryAction::Merge(reference),
                        cx,
                    );
                });
            }
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.rebase_into").to_string(),
        IconName::GitCompareArrows,
        {
            let rebase_app = app.clone();
            let rebase_name = name.clone();
            move |_, _, cx| {
                let upstream = rebase_name.clone();
                rebase_app.update(cx, |this, cx| {
                    this.git_history_on_current(
                        crate::git::actions::GitHistoryAction::Rebase(upstream),
                        cx,
                    );
                });
            }
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.menu.update").to_string(),
        IconName::RotateCw,
        move |_, _, cx| {
            app_pull.update(cx, |this, cx| this.git_pull_current(cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.push").to_string(),
        IconName::ArrowUpFromLine,
        move |_, _, cx| {
            app_push.update(cx, |this, cx| this.git_push_current(cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.menu.new_worktree").to_string(),
        IconName::FolderGit,
        move |_, window, cx| {
            let branch = worktree_name.clone();
            // 表单要建 InputState，需要窗口句柄：菜单回调本身就带 window，
            // 直接往下传即可，不必绕 update_in（那个只服务异步上下文）。
            app_worktree.update(cx, |this, cx| {
                this.git_new_worktree_from(&branch, window, cx);
            });
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.menu.delete").to_string(),
        IconName::Trash,
        move |_, _, cx| {
            let name = delete_name.clone();
            app.update(cx, |this, cx| this.git_delete_ref(&name, cx));
        },
    ))
}

/// 提交右键菜单的目标包：标签、提交号、父提交与分支态一次带齐，
/// 平铺入参已超过规范的 5 个上限。
pub(crate) struct CommitMenuTarget {
    pub(crate) tab_id: TabId,
    pub(crate) oid: String,
    pub(crate) parents: Vec<String>,
    pub(crate) on_branch: bool,
    /// 这一行是否可以被「压缩到／优选到父提交」。
    pub(crate) can_fold: bool,
    /// 这一行是否就是分支头提交：它之上没有可重排的提交，不能作为变基基线。
    pub(crate) is_tip: bool,
}

/// 提交行右键菜单：与 IDEA 的提交上下文菜单同序——复制、从此提交新建分支/标签、
/// 转到父/子提交、拣选、撤销提交、创建补丁、在浏览器中查看、推送此前所有提交，
/// 压缩到／优选到父提交（仅分支头提交可用），最后是「重置到此处」的三档力度。
///
/// 「转到父提交」在根提交上置灰而不是隐藏：用户看得到这条能力存在，
/// 只是当前这一行没有父提交。
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
    } = target;
    let branch_oid = oid.clone();
    let tag_oid = oid.clone();
    let revert_oid = oid.clone();
    let pick_oid = oid.clone();
    let copy_oid = oid.clone();
    let mut menu = menu
        .item(menu_item(
            rust_i18n::t!("git.menu.copy_oid").to_string(),
            IconName::Copy,
            {
                let copy_app = app.clone();
                let copy_oid = copy_oid.clone();
                move |_, _, cx| {
                    let oid = copy_oid.clone();
                    copy_app.update(cx, |this, cx| {
                        this.write_clipboard(&oid, cx);
                    });
                }
            },
        ))
        .item(menu_item(
            rust_i18n::t!("git.menu.new_branch_from").to_string(),
            IconName::GitBranchPlus,
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
        .item(
            MenuItem::new(
                rust_i18n::t!("git.menu.jump_parent").to_string(),
                IconName::ArrowUpToLine,
            )
            .disabled(parents.is_empty())
            .render({
                let jump_app = app.clone();
                let jump_oid = oid.clone();
                move |_, _, cx| {
                    let target = jump_oid.clone();
                    jump_app.update(cx, |this, cx| {
                        this.git_jump_commit(tab_id, LogJump::Parent(target), cx);
                    });
                }
            }),
        )
        .item(menu_item(
            rust_i18n::t!("git.menu.jump_child").to_string(),
            IconName::ArrowDownToLine,
            {
                let jump_app = app.clone();
                let jump_oid = oid.clone();
                move |_, _, cx| {
                    let target = jump_oid.clone();
                    jump_app.update(cx, |this, cx| {
                        this.git_jump_commit(tab_id, LogJump::Child(target), cx);
                    });
                }
            },
        ))
        .item(PopupMenuItem::separator())
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
        .item(PopupMenuItem::separator())
        .item(menu_item(
            rust_i18n::t!("git.menu.create_patch").to_string(),
            IconName::Download,
            {
                let patch_app = app.clone();
                let patch_oid = oid.clone();
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
        .item(menu_item(
            rust_i18n::t!("git.menu.browse").to_string(),
            IconName::Globe,
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
        .item(
            // 变基基线是选中的提交本身，因此头提交之上没有可重排的提交时置灰。
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
        .item(PopupMenuItem::separator())
        .item(
            // 折叠只作用于「分支头提交压进父提交」：选中别的行时置灰而不是隐藏，
            // 用户看得到这条能力，也知道为什么现在点不动。
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
        .item(PopupMenuItem::separator());
    // 三档重置力度并列，破坏性最强的硬重置放最后，与 IDEA 的确认框同序。
    for (key, kind) in [
        ("git.reset.soft", transom_core::GitResetKind::Soft),
        ("git.reset.mixed", transom_core::GitResetKind::Mixed),
        ("git.reset.hard", transom_core::GitResetKind::Hard),
    ] {
        let reset_app = app.clone();
        let reset_oid = oid.clone();
        menu = menu.item(menu_item(
            rust_i18n::t!(key).to_string(),
            IconName::RotateCcw,
            move |_, _, cx| {
                let target = reset_oid.clone();
                reset_app.update(cx, |this, cx| {
                    this.git_history_on_current(
                        crate::git::actions::GitHistoryAction::Reset { target, kind },
                        cx,
                    );
                });
            },
        ));
    }
    menu
}
