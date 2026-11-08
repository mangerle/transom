//! 容器工作区发给外壳的意图事件。
//!
//! 设计原理（Why）：工作区头部、列表工具条与列表行只读取实体状态，需要窗口级
//! 能力（全屏覆盖层、引擎弹窗、输入表单）时不能反向持有外壳句柄。实体在创建时
//! 由外壳订阅一次，渲染函数只负责 `emit`；外壳处理事件时按订阅创建时捕获的标签
//! id 定位，不反查发出事件的实体，保持「数据向下、意图向上」的单向依赖。
//!
//! 代价与局限：事件只表达意图，实际状态仍在工作区实体与 `ContainerUiState` 里；
//! 新增需要外壳动作的按钮时必须同时补一个变体与处理分支，遗漏会表现为点击无响应。
//! 需要 `Window` 或不能在事件派发期执行的动作由外壳推进 `window_requests`，
//! 在渲染期统一执行。

use super::ContainerWorkspace;
use super::types::{
    ContainerDetailTab, ContainerPruneTarget, ContainerSection, RowAction, RowTarget,
};
use crate::app::AppShell;
use gpui_kit::{Context, EventEmitter};
use std::path::PathBuf;
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
    /// Compose YAML 已读取完成；外壳需要用 `Window` 创建编辑器并写回工作区。
    ///
    /// 设计原理（Why）：`TextareaState` 的创建必须拿到 `Window`，而读盘发生在
    /// 工作区实体的异步任务里；事件只携带已读出的文本与定位信息，外壳在渲染期
    /// 创建编辑器，避免在实体派发阶段同步开窗或重复读盘。
    ComposeYamlLoaded {
        /// 项目名。
        project: String,
        /// 读取到的 YAML 配置文件路径。
        path: PathBuf,
        /// 文件全文。
        text: String,
    },
    /// Compose YAML 读取失败；外壳在渲染期弹出失败通知。
    ComposeYamlReadFailed(String),
    /// 为容器端口建立 SSH 本地端口转发；需要窗口打开弹窗。
    ///
    /// 容器名、私有端口与已发布端口全部随事件携带，外壳处理时不回读工作区实体。
    OpenPortForward {
        /// 容器名（已去掉 Docker 的 `/` 前缀）。
        container: String,
        /// 容器内私有端口。
        private: u16,
        /// 已发布到宿主机的端口；`None` 表示直接用私有端口。
        public: Option<u16>,
    },
    /// 打开数据卷详情对应的文件浏览器。
    ///
    /// 卷详情只显示「浏览文件」入口；切到文件页签与拉取根目录都要走外壳既有的
    /// 目录加载链路，因此以事件上报，由外壳在渲染期执行。
    OpenVolumeFiles,
    /// 打开容器资源配置更新表单；容器名随事件携带。
    OpenContainerUpdate(String),
    /// 打开容器配置反向导出弹窗；容器名随事件携带。
    OpenContainerExport(String),
    /// 打开容器提交镜像表单；容器名随事件携带。
    OpenContainerCommit(String),
    /// 打开镜像打标签输入条；镜像 ID 随事件携带。
    OpenImageTag(String),
    /// 打开镜像推送弹窗；镜像 ID 与源引用随事件携带。
    ///
    /// 源引用用于拆分目标仓库与标签，事件自带全部参数，外壳处理时不回读工作区。
    OpenImagePush {
        /// 镜像 ID。
        id: String,
        /// 源镜像引用（含 tag）。
        reference: String,
    },
    /// 把镜像另存为本地 Tar 归档；镜像 ID 随事件携带。
    SaveImage(String),
    /// 打开网络连接容器输入条；网络 ID 随事件携带。
    OpenNetworkConnect(String),
    /// 以既有容器为模板打开克隆运行表单。
    CloneContainer {
        /// 源容器 id。
        id: String,
        /// 源容器名。
        name: String,
    },
    /// 以既有容器配置重建容器。
    RecreateContainer {
        /// 源容器 id。
        id: String,
        /// 源容器名。
        name: String,
    },
    /// 切换到指定详情页签；终端等页签需要外壳初始化会话。
    SetDetailTab(ContainerDetailTab),
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
    /// 处理函数不反查容器实体来确定归属，也不读它的状态。不需要窗口且只改外壳
    /// 状态的动作直接调用既有外壳方法；需要窗口（输入表单、弹窗）或必须在实体
    /// 派发结束之后才能读写的动作推进 `window_requests`，由渲染期统一执行。
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
            ContainerWorkspaceEvent::SaveImage(image) => {
                self.save_image(tab_id, image.clone(), cx);
            }
            // 打开输入表单与弹窗需要 `Window`，事件派发阶段拿不到；行动作里的
            // 详情页切换、复制与生命周期也统一交给渲染期执行，避免两套分发逻辑。
            ContainerWorkspaceEvent::OpenRunForm(_)
            | ContainerWorkspaceEvent::OpenImageHub
            | ContainerWorkspaceEvent::OpenCreateDialog(_)
            | ContainerWorkspaceEvent::OpenBuildDialog
            | ContainerWorkspaceEvent::ComposeYamlLoaded { .. }
            | ContainerWorkspaceEvent::ComposeYamlReadFailed(_)
            | ContainerWorkspaceEvent::OpenPortForward { .. }
            | ContainerWorkspaceEvent::OpenVolumeFiles
            | ContainerWorkspaceEvent::OpenContainerUpdate(_)
            | ContainerWorkspaceEvent::OpenContainerExport(_)
            | ContainerWorkspaceEvent::OpenContainerCommit(_)
            | ContainerWorkspaceEvent::OpenImageTag(_)
            | ContainerWorkspaceEvent::OpenImagePush { .. }
            | ContainerWorkspaceEvent::OpenNetworkConnect(_)
            | ContainerWorkspaceEvent::CloneContainer { .. }
            | ContainerWorkspaceEvent::RecreateContainer { .. }
            | ContainerWorkspaceEvent::SetDetailTab(_)
            | ContainerWorkspaceEvent::RunRowAction { .. } => {
                self.state
                    .window_requests
                    .push_container(tab_id, event.clone());
                cx.notify();
            }
        }
    }
}
