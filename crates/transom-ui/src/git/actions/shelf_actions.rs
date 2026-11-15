//! 工作树与搁置（stash）的实体内聚动作。
//!
//! 设计原理（Why）：工作树列表刷新与创建均属于特定 `GitWorkspace` 的领域行为，
//! 直接通过实体自持的 `command_tx` 发送命令并在队列满时通过 `GitWorkspaceEvent::Error` 上报，
//! 彻底剥离对全局主壳层的依赖。

use crate::git::{GitWorkspace, GitWorkspaceEvent};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{GitCommand, GitRequestId};

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
