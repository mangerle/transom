//! 容器工作区持有的局部弹窗状态集合。
//!
//! 设计原理（Why）：
//! - 将原本平铺在 [`super::ContainerWorkspaceState`] 上的十余个弹窗字段聚合为独立子状态，
//!   使工作区主体状态保持职责清晰（仅保留列表、详情、日志、统计、终端与文件数据），
//!   同时让弹窗的批量清理（如切换导航分类）与统一分发更内聚。

use super::super::*;

/// 单个容器引擎工作区内挂靠的模态弹窗状态集合。
#[derive(Default)]
pub(crate) struct ContainerWorkspaceModals {
    /// 待确认的删除或危险操作。
    pub(crate) confirm: Option<ContainerConfirm>,
    /// 容器改名条：容器 ID 与输入框；`None` 表示不显示。
    pub(crate) rename_container: Option<(String, Entity<InputState>)>,
    /// 容器配置更新条：容器名与 (内存MB输入框, CPU输入框)；`None` 表示不显示。
    pub(crate) update_container: Option<(String, Entity<InputState>, Entity<InputState>)>,
    /// 镜像打标签条：镜像 ID 与新 tag 输入框；`None` 表示不显示。
    pub(crate) tag_image: Option<(String, Entity<InputState>)>,
    /// 网络连接容器条：网络 ID 与待连接容器名输入框；`None` 表示不显示。
    pub(crate) connect_network: Option<(String, Entity<InputState>)>,
    /// 容器配置导出条：(容器名, run命令, compose YAML, 是否展示compose)；`None` 表示不显示。
    pub(crate) export_config: Option<(String, String, String, bool)>,
    /// 容器提交镜像条；`None` 表示不显示。
    pub(crate) commit_container: Option<ContainerCommitState>,
    /// 镜像市场探索弹窗状态。
    pub(crate) hub_dialog: Option<ContainerHubDialog>,
    /// 镜像推送到远端仓库弹窗状态。
    pub(crate) push_dialog: Option<ContainerPushDialog>,
    /// 从 Dockerfile 构建镜像弹窗状态。
    pub(crate) build_dialog: Option<ContainerBuildDialog>,
    /// 容器端口建立本地端口转发弹窗状态。
    pub(crate) forward_dialog: Option<ContainerForwardDialog>,
    /// 全局系统磁盘用量看板弹窗开关。
    pub(crate) system_df_dialog_open: bool,
    /// Podman Pod 列表弹窗开关。
    pub(crate) pods_dialog_open: bool,
    /// Podman Quadlet 单元列表弹窗开关。
    pub(crate) quadlets_dialog_open: bool,
}

impl ContainerWorkspaceModals {
    /// 切换工作区分类时收起依赖当前选中对象上下文的轻量弹窗。
    pub(crate) fn clear_on_section_change(&mut self) {
        self.confirm = None;
        self.rename_container = None;
        self.export_config = None;
        self.commit_container = None;
    }
}
