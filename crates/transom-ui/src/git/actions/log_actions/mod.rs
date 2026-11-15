//! 日志视图的动作：过滤、分页、选行、跳转、列开关、分组勾选与差异打开。
//!
//! 设计原理（Why）：这些都是「只改视图状态 + 发读命令」的轻动作，放一处便于核对
//! 借用边界（先取数据结束借用，再投递命令）与请求登记口径。日志分页的核心状态机
//! 已迁到 `GitWorkspace::load_log`，外壳只负责定位实体与在实体更新后请求重绘。

use crate::app::AppShell;
use crate::git::commit_modal::ChangeGroup;
use crate::git::commit_modal::dir_tree::{build_dir_tree, collect_dir_keys};
use crate::git::types::GitDiffRequest;
use crate::git::{GitWorkspace, GitWorkspaceEvent};
use gpui_kit::*;
use transom_core::{
    ChangeCode, GitAction, GitCommand, GitRequestId, GitSide, LogScope, StatusEntry, StatusRecord,
    TabId,
};

impl GitWorkspace {
    /// 分页加载提交历史并上报队列满错误；`skip` 为 0 时整页替换，否则追加。
    pub(crate) fn load_log_notified(&mut self, skip: usize, cx: &mut Context<Self>) {
        if !self.load_log(skip) {
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("error.command_queue_full").to_string(),
            ));
        }
        cx.notify();
    }

    /// 读取输入框里的过滤条件并重查日志。
    pub(crate) fn apply_filter_inputs(&mut self, cx: &mut Context<Self>) {
        let search = self.state.search_input.read(cx).value().trim().to_string();
        let author = self.state.author_input.read(cx).value().trim().to_string();
        self.state.query.search = (!search.is_empty()).then_some(search);
        self.state.query.author = (!author.is_empty()).then_some(author);
        self.state.log.scope = self.state.query.scope.clone();
        self.load_log_notified(0, cx);
    }

    /// 切换正则 / 大小写开关后立刻重查。
    pub(crate) fn toggle_filter_flag(&mut self, regex: bool, cx: &mut Context<Self>) {
        let query = &mut self.state.query;
        if regex {
            query.regex = !query.regex;
        } else {
            query.case_sensitive = !query.case_sensitive;
        }
        self.apply_filter_inputs(cx);
    }

    /// 选择日志范围（当前分支 / 全部引用 / 具体引用）。
    pub(crate) fn set_log_scope(&mut self, scope: LogScope, cx: &mut Context<Self>) {
        self.state.query.scope = scope;
        self.state.log.scope = self.state.query.scope.clone();
        self.load_log_notified(0, cx);
    }

    /// 选择日期范围。
    pub(crate) fn set_log_dates(&mut self, since: Option<String>, cx: &mut Context<Self>) {
        self.state.query.since = since;
        self.load_log_notified(0, cx);
    }

    /// 按路径过滤历史。
    pub(crate) fn set_log_path(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.state.query.paths = path.into_iter().collect();
        self.load_log_notified(0, cx);
    }

    /// 切换改动树的「平铺 / 按目录」分组依据。
    pub(crate) fn toggle_group_by_directory(&mut self, cx: &mut Context<Self>) {
        self.state.group_by_directory = !self.state.group_by_directory;
        cx.notify();
    }

    /// 全部展开：清空分组与目录的折叠集合。
    pub(crate) fn expand_all_groups(&mut self, cx: &mut Context<Self>) {
        self.state.collapsed_groups.clear();
        self.state.collapsed_dirs.clear();
        cx.notify();
    }

    /// 全部收起：收起所有分组；按目录模式下把当前每个目录也收起。
    pub(crate) fn collapse_all_groups(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.state;
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
            let mut grouped: [Vec<StatusEntry>; 4] = Default::default();
            for entry in &status.entries {
                let group = match entry.record {
                    StatusRecord::Tracked { index, worktree } => {
                        if index != ChangeCode::Unmodified {
                            Some(0)
                        } else if worktree != ChangeCode::Unmodified {
                            Some(1)
                        } else {
                            None
                        }
                    }
                    StatusRecord::Conflict => Some(0),
                    StatusRecord::Untracked => Some(2),
                    StatusRecord::Ignored => Some(3),
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
        cx.notify();
    }

    /// 折叠 / 展开按目录模式下的一个目录。
    pub(crate) fn toggle_dir(&mut self, key: &str, cx: &mut Context<Self>) {
        let collapsed_dirs = &mut self.state.collapsed_dirs;
        if !collapsed_dirs.insert(key.to_string()) {
            collapsed_dirs.remove(key);
        }
        cx.notify();
    }

    /// 折叠或展开日志右侧变更文件树中的目录。
    pub(crate) fn toggle_log_file_dir(&mut self, key: &str, cx: &mut Context<Self>) {
        let collapsed_dirs = &mut self.state.collapsed_log_dirs;
        if !collapsed_dirs.insert(key.to_string()) {
            collapsed_dirs.remove(key);
        }
        cx.notify();
    }

    /// 折叠 / 展开改动树的一个分组。
    pub(crate) fn toggle_group(&mut self, key: &str, cx: &mut Context<Self>) {
        let collapsed_groups = &mut self.state.collapsed_groups;
        if !collapsed_groups.insert(key.to_string()) {
            collapsed_groups.remove(key);
        }
        cx.notify();
    }

    /// 分组头勾选：整组全选 / 全不选。
    pub(crate) fn set_group_checked(
        &mut self,
        paths: &[String],
        checked: bool,
        cx: &mut Context<Self>,
    ) {
        let checked_set = &mut self.state.checked;
        for path in paths {
            if checked {
                checked_set.insert(path.clone());
            } else {
                checked_set.remove(path);
            }
        }
        cx.notify();
    }

    /// 暂存 / 取消暂存 / 丢弃 / 删除未跟踪。
    pub(crate) fn run_paths(
        &mut self,
        action: GitAction,
        paths: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let project = self.state.project.clone();
        let command = match action {
            GitAction::Stage => GitCommand::Stage { project, paths },
            GitAction::Unstage => GitCommand::Unstage { project, paths },
            GitAction::Discard => GitCommand::Discard { project, paths },
            GitAction::DeleteUntracked => GitCommand::DeleteUntracked { project, paths },
            other => {
                log::error!("按路径批量执行的 Git 动作不匹配，动作: {other:?}");
                return;
            }
        };
        self.send(command);
        cx.notify();
    }
}

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

pub(crate) mod navigation;
