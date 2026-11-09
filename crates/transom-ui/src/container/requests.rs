//! 容器请求登记的反查与消费。
//!
//! 设计原理（Why）：请求登记在标签页的容器实体内部，事件只带 `request_id`；
//! 反查属于「应用状态 + 实体读取」的路由动作，放在 [`AppState`] 上而不是外壳，
//! 避免外壳继续承担请求反查职责。标签关闭后实体释放，迟到结果自然丢弃。

use super::*;

impl AppState {
    /// 按请求 id 取走它所属的容器标签页。
    ///
    /// 请求登记在各标签页的容器实体内部，事件只带 `request_id`，因此必须逐个
    /// 标签反查。命中即 `take`，保证同一条结果只被处理一次；标签关闭后实体随之
    /// 释放，这里查不到任何标签页，迟到结果被自然丢弃。
    pub(crate) fn take_container_request(
        &mut self,
        request_id: ContainerRequestId,
        cx: &mut App,
    ) -> Option<TabId> {
        for tab in self.workspace.tabs.iter() {
            let Some(workspace) = tab.containers() else {
                continue;
            };
            if !workspace.read(cx).has_request(request_id) {
                continue;
            }
            if workspace.update(cx, |workspace, _| workspace.take_request(request_id)) {
                return Some(tab.id);
            }
        }
        None
    }

    /// 只读查找请求所属的容器标签页；不消费登记。
    ///
    /// 用于同一请求会产生多条结果的事件（日志块、统计帧、传输进度）：这些事件
    /// 必须反复定位标签页，但不能取走登记，最终收尾事件才调用
    /// [`Self::take_container_request`]。
    pub(crate) fn container_request_tab(
        &self,
        request_id: ContainerRequestId,
        cx: &App,
    ) -> Option<TabId> {
        self.workspace.tabs.iter().find_map(|tab| {
            let workspace = tab.containers()?;
            workspace.read(cx).has_request(request_id).then_some(tab.id)
        })
    }

    /// 只读取回请求的动作描述；必须在 `take_container_request` 之前调用。
    ///
    /// 结果事件需要「启动 web-1 成功」这类文案，而取走登记后描述就不在了，
    /// 所以收尾路径的顺序固定为「先取 label，再取 tab」。
    pub(crate) fn container_request_label(
        &self,
        request_id: ContainerRequestId,
        cx: &App,
    ) -> Option<String> {
        self.workspace.tabs.iter().find_map(|tab| {
            tab.containers()
                .and_then(|workspace| workspace.read(cx).request_label(request_id))
        })
    }
}
