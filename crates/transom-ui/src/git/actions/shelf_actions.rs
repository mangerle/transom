//! 工作树与搁置（stash）的实体内聚动作。
//!
//! 设计原理（Why）：工作树列表刷新与创建均属于特定 `GitWorkspace` 的领域行为，
//! 直接通过实体自持的 `command_tx` 发送命令并在队列满时通过 `GitWorkspaceEvent::Error` 上报，
//! 彻底剥离对全局主壳层的依赖。

use crate::git::{GitCommitTab, GitWorkspace, GitWorkspaceEvent};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    GitCommand, GitProject, GitRequestId, ShelfEntry, StashEntry, StashFile, WorktreeEntry,
};

/// 新建工作树的参数对象，避免方法参数平铺过多。
pub(crate) struct CreateWorktreeParams {
    /// 目标工作树目录（支持绝对路径或相对当前仓库父目录的路径）。
    pub path: String,
    /// 要检出或新建的分支名。
    pub branch: String,
    /// 可选起点引用。
    pub start: Option<String>,
    /// 创建成功后是否在项目列表自动打开该工作树。
    pub checkout: bool,
}

impl GitWorkspace {
    /// 工作树列表落地。
    pub(crate) fn apply_worktrees_loaded(
        &mut self,
        request_id: GitRequestId,
        entries: Vec<WorktreeEntry>,
    ) {
        let state = &mut self.state;
        state.drop_request(request_id);
        state.worktrees = entries;
    }

    /// 暂存栈列表落地。
    pub(crate) fn apply_stashes_loaded(
        &mut self,
        request_id: GitRequestId,
        entries: Vec<StashEntry>,
    ) {
        let state = &mut self.state;
        state.drop_request(request_id);
        state.stashes = entries;
    }

    /// 暂存栈文件清单落地；选择器已切换时丢弃迟到结果。
    pub(crate) fn apply_stash_files_loaded(
        &mut self,
        request_id: GitRequestId,
        selector: &str,
        files: Vec<StashFile>,
    ) {
        let state = &mut self.state;
        state.drop_request(request_id);
        state.stash_files_loading = false;
        // 迟到的清单不能写进已经换过展开项的视图：编号会随删除前移，
        // 对不上选择器就如实丢弃，而不是让界面挂着一份别人的文件。
        if state.expanded_stash.as_deref() == Some(selector) {
            state.stash_files = files;
        }
    }

    /// 搁置项列表落地，并清理指向已消失条目的展开态。
    pub(crate) fn apply_shelves_loaded(
        &mut self,
        request_id: GitRequestId,
        shelves: Vec<ShelfEntry>,
    ) {
        let state = &mut self.state;
        state.drop_request(request_id);
        state.shelves = shelves;
        // 展开项可能已在别处删除：保留指向不存在条目的展开态会让面板空白。
        if state
            .expanded_shelf
            .as_ref()
            .is_some_and(|id| !state.shelves.iter().any(|entry| &entry.id == id))
        {
            state.expanded_shelf = None;
        }
    }

    /// 新建搁置项落地：去重后插到列表最前，切到搁置页并清空勾选。
    ///
    /// 返回项目快照供桥接触发刷新；状态本身不再由桥接改写。
    pub(crate) fn apply_shelved(
        &mut self,
        request_id: GitRequestId,
        entry: ShelfEntry,
    ) -> GitProject {
        let state = &mut self.state;
        state.drop_request(request_id);
        // 列表刷新事件可能已带回同一条目：按 ID 去重后再插到最前。
        state.shelves.retain(|shelf| shelf.id != entry.id);
        state.shelves.insert(0, entry);
        state.checked.clear();
        state.commit_tab = GitCommitTab::Shelf;
        state.project.clone()
    }

    /// 应用搁置项后的列表收尾：`pop` 且清理成功时移除条目并收起展开态。
    pub(crate) fn apply_shelf_applied(
        &mut self,
        shelf_id: &str,
        pop: bool,
        cleanup_failed: bool,
    ) -> GitProject {
        let state = &mut self.state;
        if pop && !cleanup_failed {
            state.shelves.retain(|shelf| shelf.id != shelf_id);
            if state.expanded_shelf.as_deref() == Some(shelf_id) {
                state.expanded_shelf = None;
            }
        }
        state.project.clone()
    }

    /// 删除搁置项后的列表收尾。
    pub(crate) fn apply_shelf_dropped(&mut self, shelf_id: &str) {
        let state = &mut self.state;
        state.shelves.retain(|shelf| shelf.id != shelf_id);
        if state.expanded_shelf.as_deref() == Some(shelf_id) {
            state.expanded_shelf = None;
        }
    }

    /// 搁置操作失败：释放在途请求占位。
    pub(crate) fn drop_shelf_request(&mut self, request_id: GitRequestId) {
        self.state.drop_request(request_id);
    }
}

impl GitWorkspace {
    /// 拉取当前项目的工作树列表。
    pub(crate) fn reload_worktrees(&mut self, cx: &mut Context<Self>) {
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        if !self.send(GitCommand::ListWorktrees {
            project: self.state.project.clone(),
            request_id,
        }) {
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("error.command_queue_full").to_string(),
            ));
        }
        cx.notify();
    }

    /// 按对话框填好的参数新建工作树。
    pub(crate) fn create_worktree(&mut self, params: CreateWorktreeParams, cx: &mut Context<Self>) {
        // 目录可能是相对仓库父目录的简写：补成绝对路径再交给 git，
        // 否则子进程的工作目录决定最终落点，用户看到的和实际创建的位置会偏。
        let target = {
            let candidate = PathBuf::from(&params.path);
            if candidate.is_absolute() {
                candidate
            } else {
                self.state
                    .project
                    .path
                    .parent()
                    .map(|dir| dir.join(&candidate))
                    .unwrap_or(candidate)
            }
        };
        let text = target.display().to_string();
        self.state.pending_worktree = Some((text.clone(), params.checkout));
        if !self.send(GitCommand::AddWorktree {
            project: self.state.project.clone(),
            path: text,
            branch: params.branch,
            start: params.start,
        }) {
            self.state.pending_worktree = None;
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("error.command_queue_full").to_string(),
            ));
        }
        cx.notify();
    }
}
