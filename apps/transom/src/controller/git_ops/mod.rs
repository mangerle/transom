//! Git 面板命令处理。
//!
//! 设计原理（Why）：
//! - 控制器不维护「已打开仓库」表：项目路径在添加时就已探测并规范化，命令
//!   自带 [`transom_core::GitProject`]，这里用 `Repo::at` 零成本构造句柄，避免每条命令再
//!   跑三趟 `rev-parse`；
//! - 读请求走「取消标志」而不是 abort：git 子进程自带超时，abort 会让管道
//!   半路废弃；界面关闭视图后迟到的结果按标志丢弃即可；
//! - 写请求统一回报 [`transom_core::GitAppEvent::ActionDone`]：动作类别由 [`transom_core::GitAction`]
//!   携带，界面据此决定刷新（暂存）还是仅提示（推送失败），不必为每个动作
//!   定义一个事件变体；
//! - 全部任务进 `git_set` JoinSet：停机时统一限时等待，不留脱缰子进程。

mod clone_op;
mod reads;
mod runner;
mod writes;

use super::state::{ControllerCtx, ControllerState};
use transom_core::GitCommand;

/// 处理 Git 面板命令。
pub(super) fn handle_git_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: GitCommand,
) {
    if let GitCommand::Clone {
        request_id,
        url,
        dest,
        depth,
    } = command
    {
        clone_op::spawn_clone(state, ctx, request_id, url, dest, depth);
        return;
    }

    if !reads::handle_read_command(state, ctx, &command) {
        writes::handle_write_command(state, ctx, command);
    }
}
