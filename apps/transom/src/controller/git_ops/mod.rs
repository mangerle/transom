//! Git 面板命令处理。
//!
//! 设计原理（Why）：
//! - 控制器不维护「已打开仓库」表：项目路径在添加时就已探测并规范化，命令
//!   自带 [`transom_core::GitProject`]，这里用当前配置快照零成本构造
//!   [`transom_git::Repo::with_config`] 句柄，避免每条命令再跑三趟 `rev-parse`；
//! - Git 可执行文件与提交作者只由 `GitCommand::Configure` 写入本控制平面持有的
//!   唯一快照，各任务派发时读取一次并绑定到仓库句柄，不存在进程级可变全局；
//! - 读请求走「取消标志」而不是 abort：git 子进程自带超时，abort 会让管道
//!   半路废弃；界面关闭视图后迟到的结果按标志丢弃即可；
//! - 写请求统一回报 [`transom_core::GitAppEvent::ActionDone`]：动作类别由 [`transom_core::GitAction`]
//!   携带，界面据此决定刷新（暂存）还是仅提示（推送失败），不必为每个动作
//!   定义一个事件变体；
//! - 离线搁置项（Shelf）例外：一条操作同时涉及 git 补丁与本地 JSON 两步，
//!   `ActionDone` 无法表达「补丁已应用但条目删除失败」的分步结果，因此使用
//!   独立的 [`transom_core::GitShelfOperation`] 与对应事件；
//! - 全部任务进 `git_set` JoinSet：停机时统一限时等待，不留脱缰子进程。

mod clone_op;
mod reads;
mod runner;
mod shelf;
mod writes;

use super::state::{ControllerCtx, ControllerState};
use transom_core::GitCommand;

/// 处理 Git 面板命令。
pub(super) fn handle_git_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: GitCommand,
) {
    if let GitCommand::Configure {
        program,
        author_name,
        author_email,
    } = &command
    {
        // 配置只由控制平面写入唯一快照；后续命令派发时各自读取一次。
        let next = transom_git::GitConfig {
            program: program.clone(),
            author_name: author_name.clone(),
            author_email: author_email.clone(),
        };
        let resolved_program = next.program().to_string();
        let author_configured =
            !next.author_name.trim().is_empty() || !next.author_email.trim().is_empty();
        *ctx.git_config.write() = next;
        log::info!(
            "Git 配置已更新，可执行文件: {resolved_program}，提交作者: {}",
            if author_configured {
                "已配置"
            } else {
                "未配置"
            }
        );
        return;
    }

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

    if shelf::handle_shelf_command(state, ctx, &command) {
        return;
    }

    if !reads::handle_read_command(state, ctx, &command) {
        writes::handle_write_command(state, ctx, command);
    }
}
