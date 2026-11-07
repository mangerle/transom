//! 日志视图的动作：过滤、分页、选行、跳转、列开关、分组勾选与差异打开。
//!
//! 设计原理（Why）：这些都是「只改视图状态 + 发读命令」的轻动作，放一处便于核对
//! 借用边界（先取数据结束借用，再 `send_command`）与请求登记口径。

use crate::app::AppShell;
use crate::git::types::GitDiffRequest;
use gpui_kit::*;
use transom_core::{GitAction, GitCommand, GitProject, GitRequestId, GitSide, TabId};

/// 日志单页条数：一屏约 40 行，5 页滚动内不必再发请求。
const LOG_PAGE_SIZE: usize = 200;

impl AppShell {
    /// 分页加载提交历史；`skip` 为 0 时整页替换，否则追加（下拉加载）。
    pub(crate) fn git_load_log(&mut self, tab_id: TabId, skip: usize, cx: &mut Context<Self>) {
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return;
        };
        let Some(state) = self.state.workspace.tabs[index].git_mut() else {
            return;
        };
        let project = state.project.clone();
        state.log.loading = true;
        state.log.error = None;
        let request_id = GitRequestId::new();
        state.pending.push(request_id);
        state.log.request = Some((request_id, skip > 0));
        let mut query = state.query.clone();
        query.skip = skip;
        query.limit = LOG_PAGE_SIZE;
        self.send_command(GitCommand::LoadLog {
            project,
            request_id,
            query,
        });
        cx.notify();
    }

    /// 读取输入框里的过滤条件并重查日志。
    pub(crate) fn git_apply_filter_inputs(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        let search = state.search_input.read(cx).value().trim().to_string();
        let author = state.author_input.read(cx).value().trim().to_string();
        state.query.search = (!search.is_empty()).then_some(search);
        state.query.author = (!author.is_empty()).then_some(author);
        state.log.scope = state.query.scope.clone();
        cx.notify();
        self.git_load_log(tab_id, 0, cx);
    }

    /// 切换正则 / 大小写开关后立刻重查。
    pub(crate) fn git_toggle_filter_flag(
        &mut self,
        tab_id: TabId,
        regex: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        if regex {
            state.query.regex = !state.query.regex;
        } else {
            state.query.case_sensitive = !state.query.case_sensitive;
        }
        cx.notify();
        self.git_apply_filter_inputs(tab_id, cx);
    }

    /// 选择日志范围（当前分支 / 全部引用 / 具体引用）。
    pub(crate) fn git_set_log_scope(
        &mut self,
        tab_id: TabId,
        scope: transom_core::LogScope,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        state.query.scope = scope;
        state.log.scope = state.query.scope.clone();
        cx.notify();
        self.git_load_log(tab_id, 0, cx);
    }

    /// 选择日期范围。
    pub(crate) fn git_set_log_dates(
        &mut self,
        tab_id: TabId,
        since: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        state.query.since = since;
        cx.notify();
        self.git_load_log(tab_id, 0, cx);
    }

    /// 按路径过滤历史。
    pub(crate) fn git_set_log_path(
        &mut self,
        tab_id: TabId,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        state.query.paths = path.into_iter().collect();
        cx.notify();
        self.git_load_log(tab_id, 0, cx);
    }

    /// 改一个差异显示选项并重算当前差异。
    pub(crate) fn git_update_diff_options(
        &mut self,
        tab_id: TabId,
        update: impl FnOnce(&mut crate::git::types::GitDiffOptions),
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        update(&mut state.diff_options);
        cx.notify();
    }

    /// 切换改动树的「平铺 / 按目录」分组依据。
    pub(crate) fn git_toggle_group_by_directory(
        &mut self,
        project: &GitProject,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_mut(project.id) else {
            return;
        };
        state.group_by_directory = !state.group_by_directory;
        cx.notify();
    }

    /// 全部展开：清空分组与目录的折叠集合。
    pub(crate) fn git_expand_all_groups(&mut self, project: &GitProject, cx: &mut Context<Self>) {
        if let Some(state) = self.git_state_of_mut(project.id) {
            state.collapsed_groups.clear();
            state.collapsed_dirs.clear();
        }
        cx.notify();
    }

    /// 全部收起：收起所有分组；按目录模式下把当前每个目录也收起。
    pub(crate) fn git_collapse_all_groups(&mut self, project: &GitProject, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_mut(project.id) else {
            return;
        };
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
            for entry in &status.entries {
                let group = match entry.record {
                    transom_core::StatusRecord::Tracked { index, worktree } => {
                        if index != transom_core::ChangeCode::Unmodified {
                            Some(ChangeGroup::Staged)
                        } else if worktree != transom_core::ChangeCode::Unmodified {
                            Some(ChangeGroup::Unstaged)
                        } else {
                            None
                        }
                    }
                    transom_core::StatusRecord::Conflict => Some(ChangeGroup::Staged),
                    transom_core::StatusRecord::Untracked => Some(ChangeGroup::Untracked),
                    transom_core::StatusRecord::Ignored => Some(ChangeGroup::Ignored),
                };
                if let Some(group) = group {
                    let dir = entry.path.rfind('/').map_or("", |at| &entry.path[..at]);
                    state
                        .collapsed_dirs
                        .insert(format!("{}|{dir}", group.key()));
                }
            }
        }
        cx.notify();
    }

    /// 折叠 / 展开按目录模式下的一个目录。
    pub(crate) fn git_toggle_dir(
        &mut self,
        project: &GitProject,
        key: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_mut(project.id) else {
            return;
        };
        if !state.collapsed_dirs.insert(key.to_string()) {
            state.collapsed_dirs.remove(key);
        }
        cx.notify();
    }

    /// 折叠 / 展开改动树的一个分组。
    pub(crate) fn git_toggle_group(
        &mut self,
        project: &GitProject,
        key: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_mut(project.id) else {
            return;
        };
        if !state.collapsed_groups.insert(key.to_string()) {
            state.collapsed_groups.remove(key);
        }
        cx.notify();
    }

    /// 分组头勾选：整组全选 / 全不选。
    pub(crate) fn git_set_group_checked(
        &mut self,
        project: &GitProject,
        paths: &[String],
        checked: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_mut(project.id) else {
            return;
        };
        for path in paths {
            if checked {
                state.checked.insert(path.clone());
            } else {
                state.checked.remove(path);
            }
        }
        cx.notify();
    }

    /// 暂存 / 取消暂存 / 丢弃 / 删除未跟踪。
    pub(crate) fn git_run_paths(
        &mut self,
        project: &GitProject,
        action: GitAction,
        paths: Vec<String>,
        cx: &mut Context<Self>,
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
        self.send_command(command);
        cx.notify();
    }

    /// 展开 / 收起左侧引用栏。
    pub(crate) fn git_toggle_refs_panel(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        state.refs_panel = !state.refs_panel;
        cx.notify();
    }

    /// 折叠 / 展开引用栏的一个分组。
    pub(crate) fn toggle_git_refs_group(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_git_tab() else {
            return;
        };
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        let default_expanded = matches!(key, "HEAD" | "git.refs.local")
            || state.refs_expanded.get(key).copied().unwrap_or(false);
        let next = match state.refs_expanded.get(key) {
            Some(current) => !current,
            None => !default_expanded,
        };
        state.refs_expanded.insert(key.to_string(), next);
        cx.notify();
    }

    /// 当前活动标签若为 Git 标签则返回它，否则退回最后一个 Git 标签。
    pub(crate) fn active_git_tab(&self) -> Option<TabId> {
        if let Some(tab_id) = self.state.workspace.active_tab
            && self.git_state_of_tab(tab_id).is_some()
        {
            return Some(tab_id);
        }
        self.state
            .workspace
            .tabs
            .iter()
            .rev()
            .find(|tab| tab.is_git())
            .map(|tab| tab.id)
    }

    /// 切换日志表的一列显示（表头右键菜单）。
    pub(crate) fn git_toggle_log_column(
        &mut self,
        tab_id: TabId,
        column: crate::git::types::LogColumn,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        column.flip(&mut state.log.columns);
        cx.notify();
    }

    /// 转到父提交 / 子提交（IDEA 提交右键菜单的两条跳转）。
    ///
    /// 设计原理（Why）：先按已加载的页就地解析，命中立刻选中；没命中才把目标
    /// 挂到 `log.jump` 上交给分页续找。跳转与分页共用同一条落地路径，
    /// 避免出现「一处能翻到、另一处永远找不到」。
    pub(crate) fn git_jump_commit(
        &mut self,
        tab_id: TabId,
        jump: crate::git::types::LogJump,
        cx: &mut Context<Self>,
    ) {
        let landed = self
            .git_state_of_tab(tab_id)
            .and_then(|state| jump.resolve(&state.log.entries));
        if let Some(index) = landed {
            self.git_select_commit_by_index(tab_id, index, cx);
            return;
        }
        let paging = self
            .git_state_of_tab_mut(tab_id)
            .map(move |state| {
                state.log.jump = Some(jump);
                state.log.has_more && !state.log.loading
            })
            .unwrap_or_default();
        if paging {
            self.git_load_more_log(tab_id, cx);
            return;
        }
        self.state.status = Some(rust_i18n::t!("git.log.jump_missing").to_string());
        cx.notify();
    }

    /// 选中一条提交（渲染层按行号回调）。
    pub(crate) fn git_select_commit_by_index(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(oid) = self
            .git_state_of_tab(tab_id)
            .and_then(|state| state.log.entries.get(index))
            .map(|entry| entry.oid.clone())
        else {
            return;
        };
        let (project, request_id) = {
            let Some(state) = self.git_state_of_tab_mut(tab_id) else {
                return;
            };
            state.selected_commit = Some(index);
            state.commit_files = Vec::new();
            state.commit_files_loading = true;
            state.diff = None;
            (state.project.clone(), GitRequestId::new())
        };
        if let Some(state) = self.git_state_of_tab_mut(tab_id) {
            state.pending.push(request_id);
        }
        self.send_command(GitCommand::LoadCommitFiles {
            project,
            request_id,
            oid,
        });
        cx.notify();
    }

    /// 加载更多历史（追加页）。
    pub(crate) fn git_load_more_log(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(len) = self
            .git_state_of_tab(tab_id)
            .map(|state| state.log.entries.len())
        else {
            return;
        };
        self.git_load_log(tab_id, len, cx);
    }

    /// 提交清单里点一个文件：与该提交的第一父比较。
    pub(crate) fn git_diff_commit_file(
        &mut self,
        tab_id: TabId,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(oid) = self
            .git_state_of_tab(tab_id)
            .and_then(|state| state.selected_entry())
            .map(|entry| entry.oid.clone())
        else {
            return;
        };
        self.git_open_diff(
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
}
