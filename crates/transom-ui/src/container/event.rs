//! 容器工作区发给外壳的意图事件。
//!
//! 设计原理（Why）：工作区头部与正文只读取实体状态，需要窗口级能力（全屏覆盖
//! 层、引擎弹窗）时不能反向持有外壳句柄。实体在创建时由外壳订阅一次，头部渲染
//! 函数只负责 `emit`；外壳处理事件时按订阅创建时捕获的标签 id 定位，不反查发出
//! 事件的实体，保持「数据向下、意图向上」的单向依赖。
//!
//! 代价与局限：事件只表达意图，实际状态仍在工作区实体与 `ContainerUiState` 里；
//! 新增需要外壳动作的按钮时必须同时补一个变体与处理分支，遗漏会表现为点击无响应。

use super::ContainerWorkspace;
use crate::app::AppShell;
use gpui_kit::{Context, EventEmitter};
use transom_core::TabId;

/// 容器工作区向外壳发出的动作意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerWorkspaceEvent {
    /// 切换当前标签的全屏覆盖层。
    ToggleFullscreen,
    /// 打开全局磁盘占用分析弹窗。
    OpenSystemDf,
    /// 打开 Podman Pod 列表弹窗。
    OpenPods,
    /// 打开 Quadlet 单元弹窗。
    OpenQuadlet,
}

impl EventEmitter<ContainerWorkspaceEvent> for ContainerWorkspace {}

impl AppShell {
    /// 处理容器工作区发来的动作意图。
    ///
    /// 设计原理（Why）：`tab_id` 由订阅创建时捕获传入，事件本身只表达动作；
    /// 处理函数不反查容器实体来确定归属，也不读它的状态。弹窗打开入口会自行
    /// 更新工作区实体（必要时才 `update`），这里只保持单向的意图处理。
    pub(crate) fn on_container_workspace_event(
        &mut self,
        tab_id: TabId,
        event: &ContainerWorkspaceEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContainerWorkspaceEvent::ToggleFullscreen => {
                self.state.containers.fullscreen =
                    if self.state.containers.fullscreen == Some(tab_id) {
                        None
                    } else {
                        Some(tab_id)
                    };
                cx.notify();
            }
            ContainerWorkspaceEvent::OpenSystemDf => {
                self.open_container_system_df_dialog(tab_id, cx);
            }
            ContainerWorkspaceEvent::OpenPods => {
                self.open_container_pods_dialog(tab_id, cx);
            }
            ContainerWorkspaceEvent::OpenQuadlet => {
                self.open_container_quadlets_dialog(tab_id, cx);
            }
        }
    }
}
