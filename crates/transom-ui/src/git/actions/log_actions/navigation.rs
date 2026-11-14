//! Git 日志引用面板、跳转与分页加载动作。

use super::*;

/// 展开 / 收起左侧引用栏。
pub(crate) fn git_toggle_refs_panel(
    shell: &mut AppShell,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        state.refs_panel = !state.refs_panel;
    });
    cx.notify();
}

/// 折叠 / 展开引用栏的一个分组。
pub(crate) fn toggle_git_refs_group(
    shell: &mut AppShell,
    key: &str,
    default_expanded: bool,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = active_git_tab(shell) else {
        return;
    };
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let expanded = &mut workspace.state.refs_expanded;
        let current = expanded.get(key).copied().unwrap_or(default_expanded);
        expanded.insert(key.to_string(), !current);
    });
    cx.notify();
}

/// 一次性展开或收起引用栏的四个引用分组。
pub(crate) fn git_set_all_ref_groups_expanded(
    shell: &mut AppShell,
    expanded: bool,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = active_git_tab(shell) else {
        return;
    };
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        let refs_expanded = &mut workspace.state.refs_expanded;
        for key in ["HEAD", "git.refs.local", "git.refs.remote", "git.refs.tags"] {
            refs_expanded.insert(key.to_string(), expanded);
        }
    });
    cx.notify();
}

/// 当前活动标签若为 Git 标签则返回它，否则退回最后一个 Git 标签。
pub(crate) fn active_git_tab(shell: &AppShell) -> Option<TabId> {
    if let Some(tab_id) = shell.state.workspace.active_tab
        && shell.state.git_workspace(tab_id).is_some()
    {
        return Some(tab_id);
    }
    shell
        .state
        .workspace
        .tabs
        .iter()
        .rev()
        .find(|tab| tab.is_git())
        .map(|tab| tab.id)
}

/// 切换日志表的一列显示（表头右键菜单）。
pub(crate) fn git_toggle_log_column(
    shell: &mut AppShell,
    tab_id: TabId,
    column: crate::git::types::LogColumn,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    workspace.update(cx, |workspace, _| {
        column.flip(&mut workspace.state.log.columns);
    });
    cx.notify();
}

/// 转到父提交 / 子提交（IDEA 提交右键菜单的两条跳转）。
///
/// 设计原理（Why）：先按已加载的页就地解析，命中立刻选中；没命中才把目标
/// 挂到 `log.jump` 上交给分页续找。跳转与分页共用同一条落地路径，
/// 避免出现「一处能翻到、另一处永远找不到」。
pub(crate) fn git_jump_commit(
    shell: &mut AppShell,
    tab_id: TabId,
    jump: crate::git::types::LogJump,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let landed = {
        let state = &workspace.read(cx).state;
        jump.resolve(&state.log.entries)
    };
    if let Some(index) = landed {
        git_select_commit_by_index(shell, tab_id, index, cx);
        return;
    }
    let paging = workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        state.log.jump = Some(jump);
        state.log.has_more && !state.log.loading
    });
    if paging {
        git_load_more_log(shell, tab_id, cx);
        return;
    }
    shell.set_transient_status(rust_i18n::t!("git.log.jump_missing"), cx);
    cx.notify();
}

/// 按提交 OID 选择提交；刷新或插入新提交后仍指向同一条记录。
pub(crate) fn git_select_commit_by_oid(
    shell: &mut AppShell,
    tab_id: TabId,
    oid: &str,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let index = workspace
        .read(cx)
        .state
        .log
        .entries
        .iter()
        .position(|entry| entry.oid == oid);
    if let Some(index) = index {
        git_select_commit_by_index(shell, tab_id, index, cx);
    }
}

/// 选中一条提交（渲染层按行号回调）。
pub(crate) fn git_select_commit_by_index(
    shell: &mut AppShell,
    tab_id: TabId,
    index: usize,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let Some(oid) = workspace
        .read(cx)
        .state
        .log
        .entries
        .get(index)
        .map(|entry| entry.oid.clone())
    else {
        return;
    };
    let (project, request_id) = workspace.update(cx, |workspace, _| {
        let state = &mut workspace.state;
        state.selected_commit = Some(index);
        state.collapsed_log_dirs.clear();
        state.commit_files = Vec::new();
        state.commit_files_loading = true;
        state.diff = None;
        let request_id = GitRequestId::new();
        state.pending.push(request_id);
        (state.project.clone(), request_id)
    });
    shell.send_command(GitCommand::LoadCommitFiles {
        project,
        request_id,
        oid,
    });
    cx.notify();
}

/// 加载更多历史（追加页）。
pub(crate) fn git_load_more_log(
    shell: &mut AppShell,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let len = workspace.read(cx).state.log.entries.len();
    super::git_load_log(shell, tab_id, len, cx);
}

/// 提交清单里点一个文件：与该提交的第一父比较。
pub(crate) fn git_diff_commit_file(
    shell: &mut AppShell,
    tab_id: TabId,
    path: &str,
    cx: &mut Context<AppShell>,
) {
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return;
    };
    let Some(oid) = workspace
        .read(cx)
        .state
        .selected_entry()
        .map(|entry| entry.oid.clone())
    else {
        return;
    };
    crate::git::actions::git_open_diff(
        shell,
        tab_id,
        GitDiffRequest {
            path: path.to_string(),
            old: GitSide::Commit(format!("{oid}^1")),
            new: GitSide::Commit(oid),
            left_label: rust_i18n::t!("git.diff.parent").to_string(),
            right_label: rust_i18n::t!("git.diff.commit").to_string(),
        },
        cx,
    );
}
