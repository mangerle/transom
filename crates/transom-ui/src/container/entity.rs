//! 容器引擎工作区实体：一个容器标签页的状态、身份与命令投递端。
//!
//! 设计原理（Why）：容器工作区的生命周期就是标签页的生命周期，状态必须随标签页
//! 一起创建与销毁，动作也只能落在它自己的状态上。把状态收进
//! `Entity<ContainerWorkspace>` 后，外壳只持有句柄，读写都经 `read` / `update`：
//! 同一标签页不会出现两份状态，也不会再出现“借用标签页状态的同时又调用外壳方法”
//! 的借用冲突。
//!
//! 代价与局限：读写都必须携带 GPUI 上下文；渲染期只允许 `read`，写动作必须在
//! `update` 闭包内完成，且闭包里不能再反查同一实体（会触发双重租借崩溃）。

use super::types::ContainerWorkspaceState;
use transom_core::{AppCommand, TabId};

/// 一个容器引擎工作区标签的实体。
pub(crate) struct ContainerWorkspace {
    /// 标签页的全部界面状态。
    pub(crate) state: ContainerWorkspaceState,
    /// 所属标签页 id：事件回传与外壳定位都用它，不必再按标签反查。
    pub(crate) tab_id: TabId,
    /// 控制器命令投递端：实体自己就能取数据，不需要外壳转手。
    pub(crate) command_tx: flume::Sender<AppCommand>,
}

impl ContainerWorkspace {
    /// 组装一个容器工作区实体。
    pub(crate) fn new(
        state: ContainerWorkspaceState,
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
                    "容器工作区投递命令失败，标签: {}，原因: {error}",
                    self.tab_id
                );
                false
            }
        }
    }
}
