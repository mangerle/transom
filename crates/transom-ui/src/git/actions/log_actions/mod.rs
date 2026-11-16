//! 日志视图的动作：过滤、分页、选行、跳转、列开关、分组勾选与差异打开。
//!
//! 设计原理（Why）：这些都是「只改视图状态 + 发读命令」的轻动作，放一处便于核对
//! 借用边界（先取数据结束借用，再投递命令）与请求登记口径。日志动作已全部内聚到
//! `GitWorkspace` 实体自身，通过事件总线向外通知状态或错误。

use crate::git::commit_modal::ChangeGroup;
use crate::git::commit_modal::dir_tree::{build_dir_tree, collect_dir_keys};
use crate::git::types::{GitDiffRequest, LogJump, MAX_JUMP_PAGES};
use crate::git::{GitWorkspace, GitWorkspaceEvent};
use gpui_kit::*;
use transom_core::{
    BranchEntry, ChangeCode, CommitFile, GitAction, GitCommand, GitRequestId, GitSide, LogPage,
    LogScope, StatusEntry, StatusRecord, TagEntry,
};

/// 日志分页落地后桥接需要执行的后续动作。
///
/// 设计原理（Why）：把「是否落地、是否继续翻页、是否放弃跳转」的判定留在实体，
/// 桥接只按结果接续 UI 动作；这样分页与跳转状态都只由实体持有。
pub(crate) struct LogLoadedResolution {
    /// 跳转目标已落地，给出其条目下标。
    pub landed: Option<usize>,
    /// 目标尚未出现且仍可继续翻页。
    pub keep_waiting: bool,
    /// 翻到页数上限仍未找到的目标，用于提示文案。
    pub missing_jump: Option<LogJump>,
}

impl GitWorkspace {
    /// 引用列表落地。
    pub(crate) fn apply_refs_loaded(
        &mut self,
        branches: Vec<BranchEntry>,
        tags: Vec<TagEntry>,
        paths: Vec<String>,
    ) {
        let state = &mut self.state;
        state.branches = branches;
        state.tags = tags;
        state.tracked_paths = paths;
    }

    /// 提交文件清单落地：按 amend 与普通提交分别登记并结束加载态。
    pub(crate) fn apply_commit_files_loaded(
        &mut self,
        request_id: GitRequestId,
        files: &[CommitFile],
    ) {
        let state = &mut self.state;
        state.drop_request(request_id);
        if state.amend_request == Some(request_id) {
            state.amend_files = files.to_vec();
            state.amend_files_loading = false;
            state.amend_request = None;
        } else {
            state.commit_files = files.to_vec();
            state.commit_files_loading = false;
        }
    }

    /// 日志分页落地；旧请求或缺失请求返回 `None`。
    pub(crate) fn apply_log_loaded(
        &mut self,
        request_id: GitRequestId,
        page: LogPage,
    ) -> Option<LogLoadedResolution> {
        let state = &mut self.state;
        state.drop_request(request_id);
        let (held, appended) = state.log.request?;
        if held != request_id {
            // 迟到的旧请求：界面已经发起了新一轮加载，直接丢弃。
            log::debug!("丢弃迟到的 Git 日志结果，请求: {request_id}");
            return None;
        }
        state.log.request = None;
        state.log.loading = false;
        state.log.error = None;
        let has_more = page.has_more;
        if appended {
            state.log.append_entries(&page.entries);
        } else {
            state.log.replace_entries(page.entries);
        }
        state.log.has_more = has_more;
        let jump = state.log.jump.clone();
        let landed = jump
            .as_ref()
            .and_then(|target| target.resolve(&state.log.entries));
        let within_budget = state.log.jump_pages < MAX_JUMP_PAGES;
        let keep_waiting =
            landed.is_none() && jump.is_some() && state.log.has_more && within_budget;
        if landed.is_some() {
            state.log.jump = None;
            state.log.jump_pages = 0;
            return Some(LogLoadedResolution {
                landed,
                keep_waiting: false,
                missing_jump: None,
            });
        }
        if keep_waiting {
            state.log.jump_pages += 1;
            return Some(LogLoadedResolution {
                landed: None,
                keep_waiting: true,
                missing_jump: None,
            });
        }
        Some(LogLoadedResolution {
            landed: None,
            keep_waiting: false,
            missing_jump: jump,
        })
    }

    /// 读取失败落地：差异槽位、日志与提交清单加载态，以及未初始化标记。
    pub(crate) fn apply_read_failure(
        &mut self,
        request_id: GitRequestId,
        message: &str,
        not_repo: bool,
    ) {
        self.diff_tasks.remove(&request_id);
        let state = &mut self.state;
        state.drop_request(request_id);
        if state.apply_diff_failed(request_id, message) {
            return;
        }
        state.log.loading = false;
        state.log.error = Some(message.to_string());
        state.commit_files_loading = false;
        if not_repo {
            // 不是仓库是「可以补初始化」的正常状态，不只是错误：
            // 工作区据此显示初始化入口，而不是只剩一句红字。
            state.not_repo = true;
            state.status_loading = false;
            state.status_error = Some(message.to_string());
        }
    }
}

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

pub(crate) mod navigation;
