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
        let Some((run_cmd, compose_yaml)) = self
            .with_container(tab_id, cx, |workspace| {
                let details = workspace.details.as_ref()?;
                Some((
                    transom_core::generate_docker_run(details),
                    transom_core::generate_docker_compose(details),
                ))
            })
            .flatten()
        else {
            return;
        };
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.export_config = Some((name, run_cmd, compose_yaml, false));
        });
        cx.notify();
    }

    /// 关闭导出弹窗。
    pub(crate) fn cancel_container_export(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.export_config = None;
        });
        cx.notify();
    }

    /// 切换导出格式（Docker Run / Docker Compose）。
    pub(crate) fn toggle_container_export_mode(
        &mut self,
        tab_id: TabId,
        is_compose: bool,
        cx: &mut Context<Self>,
    ) {
        self.with_container_mut(tab_id, cx, |workspace| {
            if let Some((_, _, _, cur)) = workspace.export_config.as_mut() {
                *cur = is_compose;
            }
        });
        cx.notify();
    }
}
