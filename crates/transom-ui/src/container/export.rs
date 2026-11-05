//! 容器配置反向导出为运行命令与 Compose 片段的状态管理。
//!
//! 设计原理（Why）：
//! - 将容器的当前配置反向导出为可重现的命令与编排片段，方便开发者备份、
//!   跨机迁移或排查参数差异；
//! - 弹窗在全局居中模态对话框展示，本模块负责状态维护与格式切换。

use super::*;
use crate::app::AppShell;

impl AppShell {
    /// 打开容器配置反向导出弹窗。
    pub(crate) fn open_container_export(
        &mut self,
        tab_id: TabId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(ref details) = workspace.details else {
            return;
        };
        let run_cmd = transom_core::generate_docker_run(details);
        let compose_yaml = transom_core::generate_docker_compose(details);
        workspace.export_config = Some((name, run_cmd, compose_yaml, false));
        cx.notify();
    }

    /// 关闭导出弹窗。
    pub(crate) fn cancel_container_export(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.export_config = None;
        }
        cx.notify();
    }

    /// 切换导出格式（Docker Run / Docker Compose）。
    pub(crate) fn toggle_container_export_mode(
        &mut self,
        tab_id: TabId,
        is_compose: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id)
            && let Some((_, _, _, ref mut cur)) = workspace.export_config
        {
            *cur = is_compose;
        }
        cx.notify();
    }
}
