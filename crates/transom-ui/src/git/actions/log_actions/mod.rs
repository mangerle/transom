//! 日志视图的动作：过滤、分页、选行、跳转、列开关、分组勾选与差异打开。
//!
//! 设计原理（Why）：这些都是「只改视图状态 + 发读命令」的轻动作，放一处便于核对
//! 借用边界（先取数据结束借用，再投递命令）与请求登记口径。日志分页的核心状态机
//! 已迁到 `GitWorkspace::load_log`，外壳只负责定位实体与在实体更新后请求重绘。

use crate::app::AppShell;
use crate::git::types::GitDiffRequest;
use gpui_kit::*;
use transom_core::{GitAction, GitCommand, GitProject, GitRequestId, GitSide, TabId};

/// 分页加载提交历史；`skip` 为 0 时整页替换，否则追加（下拉加载）。
pub(crate) fn git_load_log(
    shell: &mut AppShell,
    tab_id: TabId,
    skip: usize,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let sent = workspace.update(cx, |workspace, _| workspace.load_log(skip));
    if !sent {
        shell
            .state
            .set_error(rust_i18n::t!("error.command_queue_full").to_string());
    }
    cx.notify();
}

/// 读取输入框里的过滤条件并重查日志。
pub(crate) fn git_apply_filter_inputs(
    shell: &mut AppShell,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let (search, author) = {
        let state = &workspace.read(cx).state;
        (
            state.search_input.read(cx).value().trim().to_string(),
            state.author_input.read(cx).value().trim().to_string(),
        )
    };
    workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        state.query.search = (!search.is_empty()).then_some(search);
        state.query.author = (!author.is_empty()).then_some(author);
        state.log.scope = state.query.scope.clone();
    });
    cx.notify();
    git_load_log(shell, tab_id, 0, cx);
}

/// 切换正则 / 大小写开关后立刻重查。
pub(crate) fn git_toggle_filter_flag(
    shell: &mut AppShell,
    tab_id: TabId,
    regex: bool,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let query = &mut workspace.state.query;
        if regex {
            query.regex = !query.regex;
        } else {
            query.case_sensitive = !query.case_sensitive;
        }
    });
    cx.notify();
    git_apply_filter_inputs(shell, tab_id, cx);
}

/// 选择日志范围（当前分支 / 全部引用 / 具体引用）。
pub(crate) fn git_set_log_scope(
    shell: &mut AppShell,
    tab_id: TabId,
    scope: transom_core::LogScope,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        state.query.scope = scope;
        state.log.scope = state.query.scope.clone();
    });
    cx.notify();
    git_load_log(shell, tab_id, 0, cx);
}

/// 选择日期范围。
pub(crate) fn git_set_log_dates(
    shell: &mut AppShell,
    tab_id: TabId,
    since: Option<String>,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        workspace.state.query.since = since;
    });
    cx.notify();
    git_load_log(shell, tab_id, 0, cx);
}

/// 按路径过滤历史。
pub(crate) fn git_set_log_path(
    shell: &mut AppShell,
    tab_id: TabId,
    path: Option<String>,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        workspace.state.query.paths = path.into_iter().collect();
    });
    cx.notify();
    git_load_log(shell, tab_id, 0, cx);
}

/// 改一个差异显示选项并同步/重算当前差异。
pub(crate) fn git_update_diff_options(
    shell: &mut AppShell,
    tab_id: TabId,
    update: impl FnOnce(&mut crate::git::types::GitDiffOptions),
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let old_ignore = workspace.update(cx, |workspace, _| {
        let old_ignore = workspace.state.diff_options.ignore_whitespace;
        update(&mut workspace.state.diff_options);
        old_ignore
    });
    crate::git::actions::git_apply_diff_options_change(shell, tab_id, old_ignore, cx);
}

/// 切换改动树的「平铺 / 按目录」分组依据。
pub(crate) fn git_toggle_group_by_directory(
    shell: &mut AppShell,
    project: &GitProject,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        state.group_by_directory = !state.group_by_directory;
    });
    cx.notify();
}

/// 全部展开：清空分组与目录的折叠集合。
pub(crate) fn git_expand_all_groups(
    shell: &mut AppShell,
    project: &GitProject,
    cx: &mut Context<AppShell>,
) {
    if let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) {
        workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.collapsed_groups.clear();
            state.collapsed_dirs.clear();
        });
    }
    cx.notify();
}

/// 全部收起：收起所有分组；按目录模式下把当前每个目录也收起。
pub(crate) fn git_collapse_all_groups(
    shell: &mut AppShell,
    project: &GitProject,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        use crate::git::commit_modal::ChangeGroup;
        for group in [
            ChangeGroup::Staged,
            ChangeGroup::Unstaged,
            ChangeGroup::Untracked,
            ChangeGroup::Ignored,
        ] {
            state.collapsed_groups.insert(group.key().to_string());
        }
        state.collapsed_dirs.clear();
        if state.group_by_directory
            && let Some(status) = state.status.as_ref()
        {
            use crate::git::commit_modal::dir_tree::{build_dir_tree, collect_dir_keys};
            let mut grouped: [Vec<transom_core::StatusEntry>; 4] = Default::default();
            for entry in &status.entries {
                let group = match entry.record {
                    transom_core::StatusRecord::Tracked { index, worktree } => {
                        if index != transom_core::ChangeCode::Unmodified {
                            Some(0)
                        } else if worktree != transom_core::ChangeCode::Unmodified {
                            Some(1)
                        } else {
                            None
                        }
                    }
                    transom_core::StatusRecord::Conflict => Some(0),
                    transom_core::StatusRecord::Untracked => Some(2),
                    transom_core::StatusRecord::Ignored => Some(3),
                };
                if let Some(idx) = group {
                    grouped[idx].push(entry.clone());
                }
            }
            for (group, entries) in [
                ChangeGroup::Staged,
                ChangeGroup::Unstaged,
                ChangeGroup::Untracked,
                ChangeGroup::Ignored,
            ]
            .into_iter()
            .zip(grouped.iter())
            {
                if !entries.is_empty() {
                    let tree = build_dir_tree(entries);
                    collect_dir_keys(group.key(), &tree, &mut state.collapsed_dirs);
                }
            }
        }
    });
    cx.notify();
}

/// 折叠 / 展开按目录模式下的一个目录。
pub(crate) fn git_toggle_dir(
    shell: &mut AppShell,
    project: &GitProject,
    key: &str,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let collapsed_dirs = &mut workspace.state.collapsed_dirs;
        if !collapsed_dirs.insert(key.to_string()) {
            collapsed_dirs.remove(key);
        }
    });
    cx.notify();
}

/// 折叠或展开日志右侧变更文件树中的目录。
pub(crate) fn git_toggle_log_file_dir(
    shell: &mut AppShell,
    tab_id: TabId,
    key: &str,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let collapsed_dirs = &mut workspace.state.collapsed_log_dirs;
        if !collapsed_dirs.insert(key.to_string()) {
            collapsed_dirs.remove(key);
        }
    });
    cx.notify();
}

/// 折叠 / 展开改动树的一个分组。
pub(crate) fn git_toggle_group(
    shell: &mut AppShell,
    project: &GitProject,
    key: &str,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let collapsed_groups = &mut workspace.state.collapsed_groups;
        if !collapsed_groups.insert(key.to_string()) {
            collapsed_groups.remove(key);
        }
    });
    cx.notify();
}

/// 分组头勾选：整组全选 / 全不选。
pub(crate) fn git_set_group_checked(
    shell: &mut AppShell,
    project: &GitProject,
    paths: &[String],
    checked: bool,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace_of_project(project.id, cx) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let checked_set = &mut workspace.state.checked;
        for path in paths {
            if checked {
                checked_set.insert(path.clone());
            } else {
                checked_set.remove(path);
            }
        }
    });
    cx.notify();
}

/// 暂存 / 取消暂存 / 丢弃 / 删除未跟踪。
pub(crate) fn git_run_paths(
    shell: &mut AppShell,
    project: &GitProject,
    action: GitAction,
    paths: Vec<String>,
    cx: &mut Context<AppShell>,
) {
    if paths.is_empty() {
        return;
    }
    let command = match action {
        GitAction::Stage => GitCommand::Stage {
            project: project.clone(),
            paths,
        },
        GitAction::Unstage => GitCommand::Unstage {
            project: project.clone(),
            paths,
        },
        GitAction::Discard => GitCommand::Discard {
            project: project.clone(),
            paths,
        },
        GitAction::DeleteUntracked => GitCommand::DeleteUntracked {
            project: project.clone(),
            paths,
        },
        other => {
            log::error!("按路径批量执行的 Git 动作不匹配，动作: {other:?}");
            return;
        }
    };
    shell.send_command(command);
    cx.notify();
}

pub(crate) mod navigation;
