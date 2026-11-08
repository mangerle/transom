//! Git 工作区实体：一个 Git 标签页的状态、身份与命令投递端。
//!
//! 设计原理（Why）：Git 工作区的生命周期就是标签页的生命周期，状态必须随标签页
//! 一起创建与销毁，动作也只能落在它自己的状态上。把状态收进
//! `Entity<GitWorkspace>` 后，外壳只持有句柄，读写都经 `read` / `update`：同一
//! 标签页不会出现两份状态，也不会再出现“借用标签页状态的同时又调用外壳方法”的
//! 借用冲突。
//!
//! 代价与局限：读写都必须携带 GPUI 上下文；渲染期只允许 `read`，写动作必须在
//! `update` 闭包内完成，且闭包里不能再反查同一实体（会触发双重租借崩溃）。

use super::types::GitWorkspaceState;
use transom_core::{AppCommand, GitCommand, GitRequestId, TabId};

/// 日志单页条数：一屏约 40 行，5 页滚动内不必再发请求。
const LOG_PAGE_SIZE: usize = 200;

/// 一个 Git 标签页的实体。
pub(crate) struct GitWorkspace {
    /// 标签页的全部界面状态。
    pub(crate) state: GitWorkspaceState,
    /// 所属标签页 id：事件回传与外壳定位都用它，不必再按项目反查。
    pub(crate) tab_id: TabId,
    /// 控制器命令投递端：实体自己就能取数据，不需要外壳转手。
    pub(crate) command_tx: flume::Sender<AppCommand>,
}

impl GitWorkspace {
    /// 组装一个 Git 工作区实体。
    pub(crate) fn new(
        state: GitWorkspaceState,
        tab_id: TabId,
        command_tx: flume::Sender<AppCommand>,
    ) -> Self {
        Self {
            state,
            tab_id,
            command_tx,
        }
    }

    /// 向控制器投递命令：非阻塞 `try_send`，失败只记 warn 日志并返回 `false`。
    ///
    /// 设计原理（Why）：命令队列满时快速失败，不阻塞 UI 线程，也不 panic。
    /// 这里刻意不写应用级错误队列——命令失败的结果由事件层统一回传，实体再报
    /// 一次会让同一故障出现两条通知。
    pub(crate) fn send(&self, command: impl Into<AppCommand>) -> bool {
        match self.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                log::warn!(
                    "Git 工作区投递命令失败，标签: {}，原因: {error}",
                    self.tab_id
                );
                false
            }
        }
    }

    /// 分页加载提交历史；`skip` 为 0 时整页替换，否则追加（下拉加载）。
    ///
    /// 设计原理（Why）：请求登记、加载态与查询快照是一组必须原子的状态改动，
    /// 放在实体上后调用方只需给出页码，不必再重复一遍“先置 loading 再发命令”
    /// 的顺序；命令走实体自己的 `command_tx`，不依赖外壳。
    pub(crate) fn load_log(&mut self, skip: usize) {
        self.state.log.loading = true;
        self.state.log.error = None;
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        self.state.log.request = Some((request_id, skip > 0));
        let mut query = self.state.query.clone();
        query.skip = skip;
        query.limit = LOG_PAGE_SIZE;
        self.send(GitCommand::LoadLog {
            project: self.state.project.clone(),
            request_id,
            query,
        });
    }
}
