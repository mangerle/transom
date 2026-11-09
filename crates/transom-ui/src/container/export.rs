//! 容器配置反向导出为运行命令与 Compose 片段的状态管理。
//!
//! 设计原理（Why）：
//! - 将容器的当前配置反向导出为可重现的命令与编排片段，方便开发者备份、
//!   跨机迁移或排查参数差异；
//! - 弹窗在全局居中模态对话框展示，本模块负责状态维护与格式切换。
//!
//! 动作归属（Why）：打开、关闭与格式切换只读写本标签页的详情快照，
//! 因此全部落在 [`ContainerWorkspace`] 上；弹窗只读取状态，不再经外壳转发。

use super::*;

impl ContainerWorkspace {
    /// 打开容器配置反向导出弹窗。
    ///
    /// 导出内容由当前容器的 inspect 详情即时生成；详情尚未加载时保持静默，
    /// 与迁移前行为一致。
    pub(crate) fn open_container_export(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(details) = self.state.details.as_ref() else {
            return;
        };
        let run_cmd = transom_core::generate_docker_run(details);
        let compose_yaml = transom_core::generate_docker_compose(details);
        self.state.export_config = Some((name, run_cmd, compose_yaml, false));
        cx.notify();
    }

    /// 关闭导出弹窗。
    pub(crate) fn cancel_container_export(&mut self, cx: &mut Context<Self>) {
        self.state.export_config = None;
        cx.notify();
    }

    /// 切换导出格式（Docker Run / Docker Compose）。
    pub(crate) fn toggle_container_export_mode(
        &mut self,
        is_compose: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, _, _, current)) = self.state.export_config.as_mut() {
            *current = is_compose;
        }
        cx.notify();
    }
}
