//! 容器工作区发给外壳的意图事件。
//!
//! 设计原理（Why）：工作区头部、列表工具条与列表行只读取实体状态，需要窗口级
//! 能力（全屏覆盖层、引擎弹窗、输入表单）时不能反向持有外壳句柄。实体在创建时
//! 由外壳订阅一次，渲染函数只负责 `emit`；外壳处理事件时按订阅创建时捕获的标签
//! id 定位，不反查发出事件的实体，保持「数据向下、意图向上」的单向依赖。
//!
//! 代价与局限：事件只表达意图，实际状态仍在工作区实体与 `ContainerUiState` 里；
//! 新增需要外壳动作的按钮时必须同时补一个变体与处理分支，遗漏会表现为点击无响应。
//! 需要 `Window` 的变体由外壳推进 `window_requests`，在渲染期统一执行。

use super::ContainerWorkspace;
use super::types::{ContainerPruneTarget, ContainerSection, RowAction, RowTarget};
use crate::app::AppShell;
use gpui_kit::{Context, EventEmitter};
use transom_core::TabId;

/// 容器工作区向外壳发出的动作意图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContainerWorkspaceEvent {
    /// 切换当前标签的全屏覆盖层。
    ToggleFullscreen,
    /// 打开全局磁盘占用分析弹窗。
    OpenSystemDf,
    /// 打开 Podman Pod 列表弹窗。
    OpenPods,
    /// 打开 Quadlet 单元弹窗。
    OpenQuadlet,
    /// 打开「运行容器」表单；图片引用为从镜像行进入时的预填值。
    OpenRunForm(Option<String>),
    /// 打开远端镜像仓库探索弹窗。
    OpenImageHub,
    /// 打开拉取/新建弹窗；分类决定字段组合。
    OpenCreateDialog(ContainerSection),
    /// 用内置模板新建本地 Compose 项目。
    CreateComposeFromTemplate(String),
    /// 导入本地 Compose YAML。
    ImportCompose,
    /// 从本地 Tar 归档导入镜像。
    LoadImage,
    /// 打开 Dockerfile 构建弹窗。
    OpenBuildDialog,
    /// 清理指定分类的未使用资源。
    PruneTarget(ContainerPruneTarget),
    /// 从本地 Tar 归档恢复数据卷；`None` 表示由文件名推导卷名。
    RestoreVolume(Option<String>),
    /// 对列表行执行动作；具体分发由外壳统一完成，其中部分动作需要窗口。
    RunRowAction {
        target: RowTarget,
        action: RowAction,
    },
}

impl EventEmitter<ContainerWorkspaceEvent> for ContainerWorkspace {}

impl AppShell {
    /// 处理容器工作区发来的动作意图。
    ///
    /// 设计原理（Why）：`tab_id` 由订阅创建时捕获传入，事件本身只表达动作；
    /// 处理函数不反查容器实体来确定归属，也不读它的状态。不需要窗口的动作直接
    /// 调用既有外壳方法；需要窗口的动作（输入表单、弹窗）推进 `window_requests`，
    /// 由渲染期统一执行，避免在事件派发阶段缺少 `Window` 时丢失动作。
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
            ContainerWorkspaceEvent::CreateComposeFromTemplate(content) => {
                self.create_local_compose_with_content(tab_id, content.clone(), cx);
            }
            ContainerWorkspaceEvent::ImportCompose => {
                self.import_local_compose(tab_id, cx);
            }
            ContainerWorkspaceEvent::LoadImage => {
                self.load_image(tab_id, cx);
            }
            ContainerWorkspaceEvent::PruneTarget(target) => {
                self.prune_container_target(tab_id, *target, true, cx);
            }
            ContainerWorkspaceEvent::RestoreVolume(volume) => {
                self.restore_volume(tab_id, volume.clone(), cx);
            }
            // 打开输入表单与弹窗需要 `Window`，事件派发阶段拿不到；行动作里的
            // 详情页切换、复制与生命周期也统一交给渲染期执行，避免两套分发逻辑。
            ContainerWorkspaceEvent::OpenRunForm(_)
            | ContainerWorkspaceEvent::OpenImageHub
            | ContainerWorkspaceEvent::OpenCreateDialog(_)
            | ContainerWorkspaceEvent::OpenBuildDialog
            | ContainerWorkspaceEvent::RunRowAction { .. } => {
                self.state
                    .window_requests
                    .push_container(tab_id, event.clone());
                cx.notify();
            }
        }
    }
}
