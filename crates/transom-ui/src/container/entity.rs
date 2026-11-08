//! 容器引擎工作区实体：一个容器标签页的状态、身份、在途请求与命令投递端。
//!
//! 设计原理（Why）：容器工作区的生命周期就是标签页的生命周期，状态必须随标签页
//! 一起创建与销毁，动作也只能落在它自己的状态上。把状态收进
//! `Entity<ContainerWorkspace>` 后，外壳只持有句柄，读写都经 `read` / `update`：
//! 同一标签页不会出现两份状态，也不会再出现“借用标签页状态的同时又调用外壳方法”
//! 的借用冲突。
//!
//! 在途请求同样归标签页所有：事件只带 `request_id`，外壳按它反查实体；标签关闭后
//! 实体随之释放，登记自然消失，迟到的结果不再落到任何标签页。
//!
//! 代价与局限：读写都必须携带 GPUI 上下文；渲染期只允许 `read`，写动作必须在
//! `update` 闭包内完成，且闭包里不能再反查同一实体（会触发双重租借崩溃）。

use super::toast::command_toast_label;
use super::types::ContainerWorkspaceState;
use std::collections::HashMap;
use transom_core::{AppCommand, ContainerCommand, ContainerRequestId, TabId};

/// 一个容器引擎工作区标签的实体。
pub(crate) struct ContainerWorkspace {
    /// 标签页的全部界面状态。
    pub(crate) state: ContainerWorkspaceState,
    /// 所属标签页 id：事件回传与外壳定位都用它，不必再按标签反查。
    pub(crate) tab_id: TabId,
    /// 控制器命令投递端：实体自己就能取数据，不需要外壳转手。
    pub(crate) command_tx: flume::Sender<AppCommand>,
    /// 在途请求登记：请求 id → 动作描述（只读请求为 `None`）。
    ///
    /// 请求归属标签页，标签身份已经由实体自身的 `tab_id` 表达，因此这里只留
    /// 动作描述。标签关闭时实体释放，登记一并消失；迟到结果按 `request_id`
    /// 查不到任何标签页，自然被丢弃。
    requests: HashMap<ContainerRequestId, Option<String>>,
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
            requests: HashMap::new(),
        }
    }

    /// 登记一条在途请求的动作描述；空描述归一为 `None`。
    ///
    /// 设计原理（Why）：结果事件只带 `request_id`，提示文案要靠发送时的命令
    /// 推导；只读请求不登记描述，成功时不弹提示，避免每次刷新都糊一条通知。
    pub(crate) fn track(&mut self, request_id: ContainerRequestId, label: Option<String>) {
        self.requests
            .insert(request_id, label.filter(|label| !label.is_empty()));
    }

    /// 取走一条请求登记，返回它此前是否存在。
    ///
    /// 事件处理必须用「取走」而不是「查询」：同一条结果只处理一次，重复事件
    /// 不应再次刷新或弹提示。
    pub(crate) fn take_request(&mut self, request_id: ContainerRequestId) -> bool {
        self.requests.remove(&request_id).is_some()
    }

    /// 取回发送时登记的动作描述（不消费记录）：结果事件用它拼提示文案。
    pub(crate) fn request_label(&self, request_id: ContainerRequestId) -> Option<String> {
        self.requests
            .get(&request_id)
            .and_then(|label| label.clone())
    }

    /// 该请求当前是否登记在本标签页上。
    pub(crate) fn has_request(&self, request_id: ContainerRequestId) -> bool {
        self.requests.contains_key(&request_id)
    }

    /// 登记请求后投递命令；投递失败时撤销登记。
    ///
    /// 返回值与 [`Self::send`] 一致：`false` 表示命令队列已满、命令没有进入
    /// 控制器。此时必须撤销登记，否则事件永远不来，这条请求会一直占着登记位。
    pub(crate) fn send_tracked(
        &mut self,
        request_id: ContainerRequestId,
        command: ContainerCommand,
    ) -> bool {
        // 发送时就登记动作描述：结果事件只带 request_id，提示要靠它说清「做了什么」。
        let label = command_toast_label(&command);
        self.track(request_id, label);
        if self.send(command) {
            return true;
        }
        self.requests.remove(&request_id);
        false
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
