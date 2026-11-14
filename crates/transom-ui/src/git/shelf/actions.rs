//! Git 搁置项（Shelf）动作与右键菜单。

use super::*;

/// 展开或折叠一个搁置项。
pub(super) fn git_toggle_shelf_expanded(
    shell: &mut AppShell,
    tab_id: TabId,
    entry_id: &str,
    cx: &mut Context<AppShell>,
) {
    if let Some(workspace) = shell.state.git_workspace(tab_id) {
        workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.expanded_shelf = if state.expanded_shelf.as_deref() == Some(entry_id) {
                None
            } else {
                Some(entry_id.to_string())
            };
        });
    }
    cx.notify();
}

/// 重新读取搁置项列表。
///
/// 读盘经命令总线交给控制器；界面只登记请求号，事件落地时整体替换列表。
pub(super) fn git_reload_shelves(
    shell: &mut AppShell,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let request_id = GitRequestId::new();
    let project = workspace.update(cx, |workspace, _| {
        workspace.state.pending.push(request_id);
        workspace.state.project.clone()
    });
    shell.send_command(GitCommand::LoadShelves {
        project,
        request_id,
    });
    cx.notify();
}

/// 把工作区勾选的改动创建为独立离线搁置项（Shelf）。
///
/// 补丁导出、工作区抽离与落盘都在控制器里完成：界面只提交「搁置意图」，
/// 失败经 [`transom_core::GitAppEvent::ShelfFailed`] 回到状态栏。
pub(crate) fn git_shelve_checked(
    shell: &mut AppShell,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let (project, files, branch) = {
        let state = &workspace.read(cx).state;
        let files: Vec<String> = if state.checked.is_empty() {
            state
                .status
                .as_ref()
                .map(|status| {
                    status
                        .entries
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            state.checked.iter().cloned().collect()
        };
        let branch = match state.status.as_ref().map(|status| &status.head) {
            Some(HeadState::OnBranch(branch)) => branch.clone(),
            Some(HeadState::Initial { branch }) => branch.clone(),
            _ => "HEAD".to_string(),
        };
        (state.project.clone(), files, branch)
    };
    if files.is_empty() {
        return;
    }
    let name = format!("{} ({})", rust_i18n::t!("git.shelf.title"), files.len());
    shell.send_command(GitCommand::Shelve {
        project,
        request_id: GitRequestId::new(),
        name,
        branch,
        files,
    });
    cx.notify();
}

/// 应用或弹回搁置项。
///
/// 补丁文本随命令携带：控制器不必再按 id 回读存储，界面手里的条目
/// 就是用户点击时看到的那一份。
pub(super) fn git_apply_shelf(
    shell: &mut AppShell,
    tab_id: TabId,
    entry_id: &str,
    pop: bool,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let (project, patch) = {
        let state = &workspace.read(cx).state;
        let Some(entry) = state.shelves.iter().find(|shelf| shelf.id == entry_id) else {
            return;
        };
        (state.project.clone(), entry.patch.clone())
    };
    shell.send_command(GitCommand::ApplyShelf {
        project,
        request_id: GitRequestId::new(),
        shelf_id: entry_id.to_string(),
        patch,
        pop,
    });
    cx.notify();
}

/// 删除指定搁置项。
fn git_drop_shelf(shell: &mut AppShell, tab_id: TabId, entry_id: &str, cx: &mut Context<AppShell>) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let project = workspace.read(cx).state.project.clone();
    shell.send_command(GitCommand::DropShelf {
        project,
        request_id: GitRequestId::new(),
        shelf_id: entry_id.to_string(),
    });
    cx.notify();
}

/// 打开搁置项中某文件的差异。
pub(super) fn git_open_shelf_file_diff(
    shell: &mut AppShell,
    tab_id: TabId,
    entry: &ShelfEntry,
    path: &str,
    cx: &mut Context<AppShell>,
) {
    crate::git::actions::git_open_diff(
        shell,
        tab_id,
        GitDiffRequest {
            path: path.to_string(),
            old: GitSide::Commit("HEAD".to_string()),
            new: GitSide::Worktree,
            left_label: format!("{} ({})", rust_i18n::t!("git.diff.baseline"), entry.branch),
            right_label: rust_i18n::t!("git.shelf.title").to_string(),
        },
        cx,
    );
}

/// 搁置项右键菜单。
pub(super) fn render_shelf_popup_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    tab_id: TabId,
    entry_id: String,
) -> PopupMenu {
    let app_apply = app.clone();
    let apply_id = entry_id.clone();
    let app_pop = app.clone();
    let pop_id = entry_id.clone();
    let drop_id = entry_id;
    menu.item(menu_item(
        rust_i18n::t!("git.shelf.apply").to_string(),
        AppIcon::Ui(UiIcon::RUN),
        move |_, _, cx| {
            let id = apply_id.clone();
            app_apply.update(cx, |this, cx| {
                git_apply_shelf(this, tab_id, &id, false, cx);
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.shelf.pop").to_string(),
        AppIcon::Ui(UiIcon::DOWNLOAD),
        move |_, _, cx| {
            let id = pop_id.clone();
            app_pop.update(cx, |this, cx| {
                git_apply_shelf(this, tab_id, &id, true, cx);
            });
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.shelf.drop").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            let id = drop_id.clone();
            app.update(cx, |this, cx| {
                git_drop_shelf(this, tab_id, &id, cx);
            });
        },
    ))
}
